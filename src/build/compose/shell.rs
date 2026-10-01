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
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) fn attach(
    context: Option<&str>,
    output: &Path,
    invocation: &[String],
    details: &Value,
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
    let attempt = format!(
        "shell-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let mut runner = Runner {
        cancellable: false,
        output,
        commands: Vec::new(),
    };
    let mut started = false;
    let mut argv = docker(context, ["exec".into(), "--interactive".into()]);
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        argv.push("--tty".into());
    }
    argv.push(id.into());
    argv.extend(invocation.iter().map(Into::into));
    let outcome = (|| -> io::Result<std::process::ExitStatus> {
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
        if inspect[0]["Config"]["Labels"]["com.docker.compose.project"].as_str()
            != Some(&resources.project)
        {
            return Err(invalid("worker does not have this build's ownership label"));
        }
        if inspect[0]["State"]["Running"] != false {
            return Err(invalid(
                "worker is already running or its state is unknown; refusing to interrupt another session",
            ));
        }
        started = true;
        runner
            .run(
                &docker(context, ["start".into(), id.into()]),
                &format!("{attempt}-start"),
                timeout,
            )
            .map_err(io::Error::other)?;
        writeln!(
            io::stderr().lock(),
            "Entering the retained Mock chroot; interactive changes are not a verified rebuild."
        )?;
        // Inherited streams preserve a real terminal (or piped agent input). No time
        // limit is imposed on user interaction; probes and recovery remain bounded.
        Command::new(&argv[0]).args(&argv[1..]).status()
    })();
    let cleanup_error = if started {
        runner
            .run(
                &docker(
                    context,
                    ["stop".into(), "--time".into(), "0".into(), id.into()],
                ),
                &format!("{attempt}-stop"),
                timeout,
            )
            .err()
    } else {
        None
    };
    let success = outcome
        .as_ref()
        .is_ok_and(std::process::ExitStatus::success)
        && cleanup_error.is_none();
    let report = json!({"format_version": 1, "operation": "shell", "success": success,
        "project": resources.project, "container_id": id,
        "argv": argv.iter().map(|s| s.to_string_lossy()).collect::<Vec<_>>(),
        "exit_code": outcome.as_ref().ok().and_then(std::process::ExitStatus::code),
        "exit_status": outcome.as_ref().ok().map(ToString::to_string),
        "error": outcome.as_ref().err().map(ToString::to_string), "cleanup_error": cleanup_error, "commands": runner.commands});
    let record = output.join("host").join(format!("{attempt}.json"));
    let saved = fs::write(
        &record,
        serde_json::to_vec_pretty(&report).map_err(io::Error::other)?,
    );
    if let Err(error) = &outcome {
        writeln!(io::stderr().lock(), "shell: {error}")?;
    }
    if let Some(error) = cleanup_error {
        writeln!(io::stderr().lock(), "shell cleanup: {error}")?;
    }
    saved?;
    writeln!(io::stderr().lock(), "shell record: {}", record.display())?;
    Ok(success)
}
