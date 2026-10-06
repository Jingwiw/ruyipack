// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Compose lifecycle and byte transfer. No RPM, Mock, or SPEC semantics live here.

pub(super) mod clean;
mod shell;

use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::{Backend, Execution, process::Runner};
use fs_err as fs;

pub(super) fn operation_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Resources {
    project: String,
    container_id: Option<String>,
    image_id: Option<String>,
    daemon_id: Option<String>,
    environment: Option<serde_json::Value>,
}

pub(super) struct Compose<'a> {
    pub context: Option<&'a str>,
    pub config: &'a Path,
    pub remove: bool,
}

pub(super) fn docker(
    context: Option<&str>,
    args: impl IntoIterator<Item = OsString>,
) -> Vec<OsString> {
    let mut command = vec!["docker".into()];
    if let Some(context) = context {
        command.extend(["--context".into(), context.into()]);
    }
    command.extend(args);
    command
}

impl Compose<'_> {
    fn compose(&self, project: &str, args: &[&str]) -> Vec<OsString> {
        let mut command = docker(
            self.context,
            [
                "compose".into(),
                "--project-directory".into(),
                self.config
                    .parent()
                    .expect("absolute config")
                    .as_os_str()
                    .to_owned(),
                "--file".into(),
                self.config.as_os_str().to_owned(),
                "--project-name".into(),
                project.into(),
            ],
        );
        command.extend(args.iter().map(OsString::from));
        command
    }
    fn prepare(
        &self,
        runner: &mut Runner<'_>,
        resources: &mut Resources,
        remaining: &impl Fn() -> Duration,
    ) -> Result<(), String> {
        runner.run(
            &docker(
                self.context,
                ["compose".into(), "version".into(), "--short".into()],
            ),
            "compose-version",
            remaining(),
        )?;
        runner.capture(
            &docker(
                self.context,
                ["context".into(), "inspect".into()]
                    .into_iter()
                    .chain(self.context.map(OsString::from)),
            ),
            "context-identity",
            remaining(),
        )?;
        let daemon_output = runner.capture(
            &docker(
                self.context,
                ["info".into(), "--format".into(), "{{json .}}".into()],
            ),
            "daemon-identity",
            remaining(),
        )?;
        let daemon: serde_json::Value = serde_json::from_str(&daemon_output)
            .map_err(|error| format!("invalid daemon identity JSON: {error}"))?;
        resources.daemon_id = Some(
            daemon
                .get("ID")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("Docker returned no daemon identity")?
                .to_owned(),
        );
        if daemon["OSType"] != "linux" {
            return Err("the build worker requires a Linux Docker daemon".into());
        }
        let config = runner.capture(
            &self.compose(&resources.project, &["config", "--format", "json"]),
            "config",
            remaining(),
        )?;
        let config: serde_json::Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
        resources.environment = Some(serde_json::json!({
            "daemon_arch": daemon["Architecture"],
            "requested_platform": config["services"]["worker"]["platform"],
            "translator": "unknown",
        }));
        runner.run(
            &self.compose(&resources.project, &["create", "--build", "worker"]),
            "create",
            remaining(),
        )?;
        let id_output = runner.capture(
            &self.compose(&resources.project, &["ps", "--all", "--quiet", "worker"]),
            "container-id",
            remaining(),
        )?;
        let id = id_output.trim().to_owned();
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Compose worker must resolve to exactly one container ID".to_owned());
        }
        resources.container_id = Some(id.clone());
        let image_output = runner.capture(
            &docker(self.context, ["inspect".into(), id.into()]),
            "container-inspect",
            remaining(),
        )?;
        let inspect: serde_json::Value = serde_json::from_str(&image_output)
            .map_err(|error| format!("invalid container inspect JSON: {error}"))?;
        let image = inspect
            .get(0)
            .and_then(|item| item.get("Image"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "container inspect returned no image identity".to_owned())?;
        if image.is_empty() {
            return Err("container inspect returned no image identity".to_owned());
        }
        resources.image_id = Some(image.to_owned());
        let image_info = runner.capture(
            &docker(
                self.context,
                ["image".into(), "inspect".into(), image.into()],
            ),
            "image-platform",
            remaining(),
        )?;
        let image_info: serde_json::Value =
            serde_json::from_str(&image_info).map_err(|e| e.to_string())?;
        let architecture = image_info[0]["Architecture"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("image architecture missing")?;
        let environment = resources
            .environment
            .as_mut()
            .expect("prepared daemon facts");
        environment["image_arch"] = architecture.into();
        if let Some(platform) = environment["requested_platform"].as_str() {
            let requested = platform
                .split('/')
                .nth(1)
                .ok_or("invalid worker platform")?;
            if requested != architecture {
                return Err("Compose platform differs from worker image architecture".into());
            }
        }
        Ok(())
    }

    fn recover(
        &self,
        runner: &mut Runner<'_>,
        resources: &Resources,
        result: &mut Execution,
        timeout: Duration,
    ) {
        // A cancellation stops work, not the bounded recovery the user requested.
        runner.cancellable = false;
        let cleanup = self.compose(
            &resources.project,
            &["down", "--volumes", "--remove-orphans"],
        );
        if let Some(id) = &resources.container_id {
            // Stop before copying: failed privileged descendants cannot keep changing
            // the evidence, and retained workers consume no running build resources.
            let stop = docker(
                self.context,
                ["stop".into(), "--timeout".into(), "0".into(), id.into()],
            );
            if let Err(error) = runner.capture(&stop, "worker-stop", timeout) {
                result.cleanup_failure = Some(error);
                if let Err(error) = runner.run(
                    &docker(self.context, ["kill".into(), id.into()]),
                    "worker-kill",
                    timeout,
                ) {
                    result.cleanup_failure = Some(format!(
                        "{}; {error}",
                        result.cleanup_failure.as_deref().unwrap_or_default()
                    ));
                    result.add_recovery_command(&stop);
                }
            }
            let copy = docker(
                self.context,
                [
                    "cp".into(),
                    format!("{id}:/output/.").into(),
                    runner.output.join("engine").into_os_string(),
                ],
            );
            if let Err(error) = runner.run(&copy, "artifact-copy", timeout) {
                result.artifact_error = Some(error);
                if !self.remove {
                    result.add_recovery_command(&copy);
                }
            }
        }
        result.cleanup_skipped = !self.remove;
        if self.remove {
            if let Err(error) = runner.run(&cleanup, "cleanup", timeout) {
                result.cleanup_failure = Some(match result.cleanup_failure.take() {
                    Some(previous) => format!("{previous}; {error}"),
                    None => error,
                });
                result.add_recovery_command(&cleanup);
            }
        } else {
            if let Some(id) = &resources.container_id {
                result.add_recovery_command(&docker(self.context, ["start".into(), id.into()]));
            }
            result.add_recovery_command(&cleanup);
        }
    }
}

impl Backend for Compose<'_> {
    fn name(&self) -> &'static str {
        "compose"
    }

    fn validate(&self) -> std::io::Result<()> {
        super::regular_file(self.config)
    }

    fn shell(
        &self,
        output: &Path,
        invocation: &[String],
        resources: &serde_json::Value,
        export: Option<&Path>,
        timeout: Duration,
    ) -> std::io::Result<bool> {
        shell::attach(self.context, output, invocation, resources, export, timeout)
    }

    fn execute(
        &self,
        previous: Option<&serde_json::Value>,
        invocation: &[String],
        staged_input: &Path,
        output: &Path,
        timeout: Duration,
    ) -> Execution {
        for directory in [output.join("host"), output.join("engine")] {
            if let Err(error) = fs::create_dir(&directory) {
                return Execution::failed(format!("create {}: {error}", directory.display()));
            }
        }
        let mut resources = match previous {
            Some(value) => match serde_json::from_value::<Resources>(value.clone()) {
                Ok(resources) => resources,
                Err(error) => {
                    return Execution::failed(format!("invalid retained environment: {error}"));
                }
            },
            None => Resources {
                project: operation_id("ruyipack"),
                container_id: None,
                image_id: None,
                daemon_id: None,
                environment: None,
            },
        };
        let mut runner = Runner {
            cancellable: true,
            output,
            commands: Vec::new(),
        };
        let start = Instant::now();
        let remaining = || timeout.saturating_sub(start.elapsed());
        // Recovery has its own bound; the build deadline cannot prevent evidence retrieval.
        let recovery_timeout = timeout.min(Duration::from_secs(60));
        let mut verified = previous.is_none();
        let primary = (|| -> Result<(), String> {
            if previous.is_some() {
                shell::verify_worker(
                    &mut runner,
                    self.context,
                    "reuse",
                    resources.container_id.as_deref().unwrap_or_default(),
                    resources.daemon_id.as_deref().unwrap_or_default(),
                    &resources.project,
                    remaining(),
                )
                .map_err(|error| error.to_string())?;
            } else {
                self.prepare(&mut runner, &mut resources, &remaining)?;
            }
            verified = true;
            let id = resources.container_id.as_ref().expect("prepared worker");
            runner.capture(
                &docker(self.context, ["start".into(), id.into()]),
                "start",
                remaining(),
            )?;
            let image_arch = resources
                .environment
                .as_ref()
                .and_then(|facts| facts["image_arch"].as_str())
                .ok_or("retained worker lacks platform evidence; clean and rebuild")?;
            let probe = runner.capture(
                &docker(
                    self.context,
                    [
                        "exec".into(),
                        "--user".into(),
                        "0".into(),
                        id.into(),
                        "python3".into(),
                        "-c".into(),
                        include_str!("probe.py").into(),
                        image_arch.into(),
                    ],
                ),
                "worker-capabilities",
                remaining(),
            )?;
            let probe: serde_json::Value =
                serde_json::from_str(&probe).map_err(|e| e.to_string())?;
            eprintln!(
                "build: target={} daemon={} image={} mount={} chroot={} translator=unknown",
                probe["target_arch"],
                resources.environment.as_ref().expect("worker facts")["daemon_arch"],
                image_arch,
                probe["mount"],
                probe["chroot"]
            );
            resources.environment.as_mut().expect("worker facts")["probe"] = probe;
            if previous.is_some() {
                runner.run(&docker(self.context, ["exec".into(), "--user".into(), "0".into(), id.into(), "python3".into(), "-c".into(),
                    "import shutil,pathlib; [(shutil.rmtree(p) if p.is_dir() and not p.is_symlink() else p.unlink()) for root in ('/input','/output') for p in pathlib.Path(root).iterdir()]".into()]),
                    "reset-attempt-files", remaining())?;
            }
            runner.run(
                &docker(
                    self.context,
                    [
                        "cp".into(),
                        staged_input.join(".").into_os_string(),
                        format!("{id}:/input").into(),
                    ],
                ),
                "input-copy",
                remaining(),
            )?;
            let mut command = docker(self.context, ["exec".into(), id.as_str().into()]);
            command.extend(invocation.iter().map(OsString::from));
            runner.run(&command, "engine", remaining())?;
            Ok(())
        })();
        let mut result = Execution {
            failure: primary.err(),
            ..Execution::default()
        };
        if verified {
            self.recover(&mut runner, &resources, &mut result, recovery_timeout);
        }
        result.success = result.failure.is_none()
            && result.artifact_error.is_none()
            && result.cleanup_failure.is_none();
        result.details = serde_json::to_value(resources)
            .expect("Compose resource metadata contains only strings");
        result.commands = runner.commands;
        result
    }
}
