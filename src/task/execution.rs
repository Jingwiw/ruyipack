// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Compose the installed command contracts with bounded execution and retained output.
use std::{
    io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

#[derive(serde::Serialize)]
struct Record<'a> {
    arguments: &'a [String],
    exit_code: Option<i32>,
    elapsed_ms: u64,
    timed_out: bool,
    interrupted: bool,
    error: Option<String>,
}

pub(super) fn run(
    executable: &Path,
    args: &[String],
    root: &Path,
    attempt: u64,
    timeout: u64,
) -> io::Result<()> {
    let stdout = root.join(format!("{attempt}.stdout.toml"));
    let stderr = root.join(format!("{attempt}.stderr.log"));
    let mut command = Command::new(executable);
    command
        .args(args)
        .args(["--format", "toml"])
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&stdout)?)
        .stderr(std::fs::File::create(&stderr)?);
    let outcome =
        crate::host_process::run(&mut command, Duration::from_secs(timeout), true, || {
            if std::fs::metadata(&stdout)?.len() > 16 * 1024 * 1024
                || std::fs::metadata(&stderr)?.len() > 512 * 1024 * 1024
            {
                return Err(io::Error::other("command output limit exceeded"));
            }
            Ok(())
        });
    let record = Record {
        arguments: args,
        exit_code: outcome.status.and_then(|s| s.code()),
        elapsed_ms: u64::try_from(outcome.elapsed_ms).unwrap_or(u64::MAX),
        timed_out: outcome.timed_out,
        interrupted: outcome.interrupted,
        error: outcome.error.as_ref().map(ToString::to_string),
    };
    crate::workspace::baseline::save(&root.join(format!("{attempt}.execution.toml")), &record)?;
    if let Some(error) = outcome.error {
        return Err(error);
    }
    let status = outcome
        .status
        .ok_or_else(|| io::Error::other("command has no exit status"))?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "{} failed; see {} and {}",
            args[0],
            stdout.display(),
            stderr.display()
        )));
    }
    Ok(())
}
