// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Container execution, resource ownership and embedded environment defaults.

use fs_err as fs;
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
pub(crate) mod compose;
pub(crate) mod process;

const FILES: &[(&str, &[u8])] = &[
    (
        "Dockerfile",
        include_bytes!("../environments/openruyi/Dockerfile"),
    ),
    (
        "compose.yaml",
        include_bytes!("../environments/openruyi/compose.yaml"),
    ),
    (
        "openruyi.cfg",
        include_bytes!("../environments/openruyi/openruyi.cfg"),
    ),
    (
        "target.json",
        include_bytes!("../environments/openruyi/target.json"),
    ),
];

pub(crate) fn write(directory: &Path) -> io::Result<()> {
    fs::create_dir(directory)?;
    for (name, bytes) in FILES {
        fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}

pub(crate) trait Backend {
    fn name(&self) -> &'static str;
    fn validate(&self) -> io::Result<()>;
    fn shell(
        &self,
        output: &Path,
        invocation: &[String],
        resources: &serde_json::Value,
        export: Option<&Path>,
        timeout: Duration,
    ) -> io::Result<bool>;
    fn execute(
        &self,
        previous: Option<&serde_json::Value>,
        invocation: &[String],
        staged_input: &Path,
        output: &Path,
        timeout: Duration,
    ) -> Execution;
}

#[derive(Serialize)]
pub(crate) struct CommandRecord {
    pub(crate) argv: Vec<String>,
    pub(crate) started: bool,
    pub(crate) exit_code: Option<i32>,
    pub(crate) exit_status: Option<String>,
    pub(crate) timed_out: bool,
    pub(crate) interrupted: bool,
    pub(crate) elapsed_ms: u128,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) error: Option<String>,
    pub(crate) termination: Option<crate::host_process::Termination>,
}

#[derive(Serialize, Default)]
pub(crate) struct Execution {
    pub(crate) success: bool,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub(crate) details: serde_json::Value,
    pub(crate) failure: Option<String>,
    pub(crate) artifact_error: Option<String>,
    pub(crate) cleanup_failure: Option<String>,
    pub(crate) cleanup_skipped: bool,
    pub(crate) recovery_commands: Vec<Vec<String>>,
    pub(crate) commands: Vec<CommandRecord>,
}

impl Execution {
    fn add_recovery_command(&mut self, argv: &[std::ffi::OsString]) {
        self.recovery_commands.push(
            argv.iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        );
    }

    pub(crate) fn failed(message: String) -> Self {
        Self {
            failure: Some(message),
            ..Self::default()
        }
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub(crate) fn regular_file(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(invalid(format!(
            "{}: expected a regular file, not a symlink or special file",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn directory(path: &Path) -> io::Result<PathBuf> {
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(invalid(format!(
            "{}: expected a directory, not a symlink",
            path.display()
        )));
    }
    fs::canonicalize(path)
}
