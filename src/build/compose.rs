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

#[derive(serde::Serialize, serde::Deserialize)]
struct Resources {
    project: String,
    container_id: Option<String>,
    image_id: Option<String>,
    daemon_id: Option<String>,
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
        timeout: Duration,
    ) -> std::io::Result<bool> {
        shell::attach(self.context, output, invocation, resources, timeout)
    }

    fn execute(
        &self,
        invocation: &[String],
        staged_input: &Path,
        output: &Path,
        timeout: Duration,
    ) -> Execution {
        let mut result = Execution::default();
        for directory in [output.join("host"), output.join("engine")] {
            if let Err(error) = fs::create_dir(&directory) {
                return Execution::failed(format!("create {}: {error}", directory.display()));
            }
        }
        let project = format!(
            "ruyipack-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut resources = Resources {
            project: project.clone(),
            container_id: None,
            image_id: None,
            daemon_id: None,
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
        let primary = (|| -> Result<(), String> {
            runner.run(
                docker(
                    self.context,
                    ["compose".into(), "version".into(), "--short".into()],
                ),
                "compose-version",
                remaining(),
            )?;
            runner.run(
                docker(
                    self.context,
                    ["context".into(), "inspect".into()]
                        .into_iter()
                        .chain(self.context.map(OsString::from)),
                ),
                "context-identity",
                remaining(),
            )?;
            let daemon_command = runner.run(
                docker(
                    self.context,
                    ["info".into(), "--format".into(), "{{json .}}".into()],
                ),
                "daemon-identity",
                remaining(),
            )?;
            let daemon: serde_json::Value =
                serde_json::from_str(&runner.stdout(daemon_command)?)
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
            runner.run(
                self.compose(&project, &["config", "--quiet"]),
                "config",
                remaining(),
            )?;
            runner.run(
                self.compose(&project, &["create", "--build", "worker"]),
                "create",
                remaining(),
            )?;
            let id_command = runner.run(
                self.compose(&project, &["ps", "--all", "--quiet", "worker"]),
                "container-id",
                remaining(),
            )?;
            let id = runner.stdout(id_command)?.trim().to_owned();
            if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("Compose worker must resolve to exactly one container ID".to_owned());
            }
            resources.container_id = Some(id.clone());
            let image_command = runner.run(
                docker(self.context, ["inspect".into(), id.clone().into()]),
                "container-inspect",
                remaining(),
            )?;
            let inspect: serde_json::Value =
                serde_json::from_str(&runner.stdout(image_command)?)
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
            runner.run(
                self.compose(&project, &["start", "worker"]),
                "start",
                remaining(),
            )?;
            runner.run(
                docker(
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
            let mut command = docker(self.context, ["exec".into(), id.into()]);
            command.extend(invocation.iter().map(OsString::from));
            runner.run(command, "engine", remaining())?;
            Ok(())
        })();
        result.failure = primary.err();
        // A cancellation stops work, not the bounded recovery the user requested.
        runner.cancellable = false;
        let cleanup = self.compose(&project, &["down", "--volumes", "--remove-orphans"]);
        if let Some(id) = &resources.container_id {
            // Stop before copying: failed privileged descendants cannot keep changing
            // the evidence, and retained workers consume no running build resources.
            let stop = docker(
                self.context,
                ["stop".into(), "--time".into(), "0".into(), id.into()],
            );
            if let Err(error) = runner.run(stop.clone(), "worker-stop", recovery_timeout) {
                result.cleanup_failure = Some(error);
                if let Err(error) = runner.run(
                    docker(self.context, ["kill".into(), id.into()]),
                    "worker-kill",
                    recovery_timeout,
                ) {
                    result.cleanup_failure = Some(format!(
                        "{}; {error}",
                        result.cleanup_failure.as_deref().unwrap_or_default()
                    ));
                    result.recovery_commands.push(
                        stop.iter()
                            .map(|arg| arg.to_string_lossy().into_owned())
                            .collect(),
                    );
                }
            }
            let copy = docker(
                self.context,
                [
                    "cp".into(),
                    format!("{id}:/output/.").into(),
                    output.join("engine").into_os_string(),
                ],
            );
            if let Err(error) = runner.run(copy.clone(), "artifact-copy", recovery_timeout) {
                result.artifact_error = Some(error);
                if !self.remove {
                    result.recovery_commands.push(
                        copy.iter()
                            .map(|arg| arg.to_string_lossy().into_owned())
                            .collect(),
                    );
                }
            }
        }
        result.cleanup_skipped = !self.remove;
        if !result.cleanup_skipped {
            if let Err(error) = runner.run(cleanup.clone(), "cleanup", recovery_timeout) {
                result.cleanup_failure = Some(match result.cleanup_failure.take() {
                    Some(previous) => format!("{previous}; {error}"),
                    None => error,
                });
                result.recovery_commands.push(
                    cleanup
                        .iter()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect(),
                );
            }
        } else {
            if let Some(id) = &resources.container_id {
                result.recovery_commands.push(
                    docker(self.context, ["start".into(), id.into()])
                        .iter()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect(),
                );
            }
            result.recovery_commands.push(
                cleanup
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            );
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
