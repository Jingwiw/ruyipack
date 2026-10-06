// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Attach only to a stopped, receipt-owned worker on its original daemon.

use super::super::{directory, invalid, process::Runner};
use super::{Resources, docker};
use fs_err as fs;
use serde_json::{Value, json};
use std::{
    io::{self, IsTerminal, Write},
    path::Path,
    process::Command,
    time::Duration,
};

pub(super) fn attach(
    context: Option<&str>,
    output: &Path,
    invocation: &[String],
    details: &Value,
    export: Option<&Path>,
    timeout: Duration,
) -> io::Result<bool> {
    let resources: Resources = serde_json::from_value(details.clone()).map_err(io::Error::other)?;
    let id = resources
        .container_id
        .as_deref()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("no retained worker; build first without --rm"))?;
    let daemon = resources
        .daemon_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("build receipt has no daemon identity"))?;
    directory(&output.join("host"))?;
    let attempt = super::operation_id("shell");
    let mut runner = Runner {
        cancellable: false,
        output,
        commands: Vec::new(),
    };
    let mut saved_patch = export
        .map(|path| {
            if path.try_exists()? {
                return Err(invalid("Patch destination already exists; not overwritten"));
            }
            tempfile::NamedTempFile::new_in(
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )
        })
        .transpose()?;
    let scripted = invocation.iter().any(|arg| arg == "--");
    let mut started = false;
    let mut command_exit_code = None;
    let mut argv = docker(context, ["exec".into(), "--interactive".into()]);
    if export.is_none() && !scripted && io::stdin().is_terminal() && io::stdout().is_terminal() {
        argv.push("--tty".into());
    }
    argv.push(id.into());
    argv.extend(invocation.iter().map(Into::into));
    let outcome = (|| -> io::Result<Option<std::process::ExitStatus>> {
        verify_worker(
            &mut runner,
            context,
            &attempt,
            id,
            daemon,
            &resources.project,
            timeout,
        )?;
        started = true;
        runner
            .capture(
                &docker(context, ["start".into(), id.into()]),
                &format!("{attempt}-start"),
                timeout,
            )
            .map_err(io::Error::other)?;
        // Inherited streams preserve a real terminal (or piped agent input). No time
        // limit is imposed on user interaction; probes and recovery remain bounded.
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        if let Some(file) = &mut saved_patch {
            command
                .stdout(file.as_file().try_clone()?)
                .stdin(std::process::Stdio::null());
        }
        if export.is_none() {
            #[derive(serde::Serialize)]
            struct Session<'a> {
                environment_may_have_changed: bool,
                argv: &'a [String],
            }
            let session = toml::to_string(&Session {
                environment_may_have_changed: true,
                argv: invocation,
            })
            .map_err(io::Error::other)?;
            crate::file_output::write_artifact(&output.join("session.toml"), session.as_bytes())
                .map_err(io::Error::other)?;
        }
        if scripted {
            let result = runner.command(&argv, &format!("{attempt}-command"), timeout);
            command_exit_code = runner.commands.last().and_then(|record| record.exit_code);
            result.map_err(io::Error::other)?;
            return Ok(None);
        }
        if export.is_some() {
            let outcome = crate::host_process::run(&mut command, timeout, false, || Ok(()));
            if let Some(error) = outcome.error {
                return Err(error);
            }
            if outcome.timed_out {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Shell operation timed out",
                ));
            }
            outcome
                .status
                .ok_or_else(|| invalid("Shell operation did not return an exit status"))
                .map(Some)
        } else {
            command.status().map(Some)
        }
    })();
    let cleanup_error = if started {
        runner
            .capture(
                &docker(
                    context,
                    ["stop".into(), "--timeout".into(), "0".into(), id.into()],
                ),
                &format!("{attempt}-stop"),
                timeout,
            )
            .err()
    } else {
        None
    };
    let success = outcome.as_ref().is_ok_and(|status| {
        status
            .as_ref()
            .is_none_or(std::process::ExitStatus::success)
    }) && cleanup_error.is_none();
    let report = json!({"format_version": 1, "operation": "shell", "success": success,
        "project": resources.project, "container_id": id,
        "argv": argv.iter().map(|s| s.to_string_lossy()).collect::<Vec<_>>(),
        "exit_code": command_exit_code.or_else(|| outcome.as_ref().ok().and_then(Option::as_ref).and_then(std::process::ExitStatus::code)),
        "exit_status": outcome.as_ref().ok().and_then(Option::as_ref).map(ToString::to_string),
        "error": outcome.as_ref().err().map(ToString::to_string), "cleanup_error": cleanup_error, "commands": runner.commands});
    let record = output.join("host").join(format!("{attempt}.json"));
    let saved = fs::write(
        &record,
        serde_json::to_vec_pretty(&report).map_err(io::Error::other)?,
    );
    if let Err(error) = &outcome {
        crate::output_cli::stderr().message(
            crate::output_cli::HumanLevel::Debug,
            None,
            format_args!("{error}"),
        )?;
        let detail = runner.commands.iter().find(|record| record.error.is_some());
        let summary = detail.and_then(|record| {
            if record.timed_out {
                Some("deadline exceeded")
            } else if record.interrupted {
                Some("interrupted")
            } else {
                record.exit_status.as_deref()
            }
        });
        crate::output_cli::stderr().message(
            crate::output_cli::HumanLevel::Error,
            None,
            format_args!(
                "shell: {}",
                summary.map_or_else(|| error.to_string(), str::to_owned)
            ),
        )?;
    }
    if let Ok(Some(status)) = &outcome
        && !status.success()
    {
        crate::output_cli::stderr().message(
            crate::output_cli::HumanLevel::Error,
            None,
            format_args!("shell: {status}"),
        )?;
    }
    if let Some(error) = cleanup_error {
        writeln!(io::stderr().lock(), "shell cleanup: {error}")?;
    }
    saved?;
    if success && let (Some(file), Some(path)) = (saved_patch, export) {
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        writeln!(
            io::stderr().lock(),
            "Patch saved: {}; declare it in recipe SPEC, then build to verify",
            path.display()
        )?;
    }
    crate::output_cli::stderr().message(
        if success {
            crate::output_cli::HumanLevel::Debug
        } else {
            crate::output_cli::HumanLevel::Info
        },
        None,
        format_args!(
            "shell record: {}",
            crate::output_cli::human_path(&record).display()
        ),
    )?;
    Ok(success)
}

pub(super) fn verify_worker(
    runner: &mut Runner<'_>,
    context: Option<&str>,
    attempt: &str,
    id: &str,
    daemon: &str,
    project: &str,
    timeout: Duration,
) -> io::Result<()> {
    let captured = runner
        .capture(
            &docker(
                context,
                ["info".into(), "--format".into(), "{{json .}}".into()],
            ),
            &format!("{attempt}-daemon"),
            timeout,
        )
        .map_err(io::Error::other)?;
    let info: Value = serde_json::from_str(&captured).map_err(io::Error::other)?;
    if info["ID"].as_str() != Some(daemon) {
        return Err(invalid(
            "Docker daemon identity differs from the build receipt",
        ));
    }
    let captured = runner
        .capture(
            &docker(context, ["inspect".into(), id.into()]),
            &format!("{attempt}-inspect"),
            timeout,
        )
        .map_err(io::Error::other)?;
    let inspect: Value = serde_json::from_str(&captured).map_err(io::Error::other)?;
    if inspect[0]["Config"]["Labels"]["com.docker.compose.project"].as_str() != Some(project) {
        return Err(invalid("worker does not have this build's ownership label"));
    }
    if inspect[0]["State"]["Running"] != false {
        return Err(invalid(
            "worker is already running or its state is unknown; refusing to interrupt another session",
        ));
    }
    Ok(())
}
