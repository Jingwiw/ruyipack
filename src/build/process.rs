// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
// SPDX-License-Identifier: MulanPSL-2.0

//! Build command logs and receipts; process lifetime is shared with workspace Git.

use super::CommandRecord;
use crate::host_process;
use fs_err as fs;
use std::{
    ffi::OsString,
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

pub(super) struct Runner<'a> {
    pub(super) cancellable: bool,
    pub(super) output: &'a Path,
    pub(super) commands: Vec<CommandRecord>,
}

impl Runner<'_> {
    /// Build output streams directly to files; it is not buffered in memory.
    pub(super) fn run(
        &mut self,
        argv: &[OsString],
        stage: &str,
        budget: Duration,
    ) -> Result<usize, String> {
        let index = self.commands.len();
        let stdout = format!("host/{index:03}-{stage}.stdout.log");
        let stderr = format!("host/{index:03}-{stage}.stderr.log");
        let mut record = CommandRecord {
            argv: argv
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            started: false,
            exit_code: None,
            exit_status: None,
            timed_out: false,
            interrupted: false,
            elapsed_ms: 0,
            stdout,
            stderr,
            error: None,
            termination: None,
        };
        let _ = writeln!(
            io::stderr().lock(),
            "build: {stage}; stdout: {}; stderr: {}",
            self.output.join(&record.stdout).display(),
            self.output.join(&record.stderr).display()
        );
        let result = (|| -> io::Result<bool> {
            let program = argv
                .first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
            let mut command = Command::new(program);
            command
                .args(&argv[1..])
                .stdin(Stdio::null())
                .stdout(fs::File::create(self.output.join(&record.stdout))?.into_file())
                .stderr(fs::File::create(self.output.join(&record.stderr))?.into_file());
            let outcome = host_process::run(&mut command, budget, self.cancellable, || Ok(()));
            record.started = outcome.started;
            record.exit_code = outcome.status.and_then(|status| status.code());
            record.exit_status = outcome.status.map(|status| status.to_string());
            record.timed_out = outcome.timed_out;
            record.interrupted = outcome.interrupted;
            record.elapsed_ms = outcome.elapsed_ms;
            record.termination = outcome.termination;
            if let Some(error) = outcome.error {
                return Err(error);
            }
            Ok(outcome.status.is_some_and(|status| status.success()))
        })();
        let failure = match result {
            Ok(true) => None,
            Ok(false) => Some(format!(
                "{stage}: {}",
                record.exit_status.as_deref().unwrap_or("process failed")
            )),
            Err(error) => Some(format!("{stage}: {error}")),
        }
        .map(|error| {
            format!(
                "{error}; stdout: {}; stderr: {}",
                self.output.join(&record.stdout).display(),
                self.output.join(&record.stderr).display()
            )
        });
        record.error.clone_from(&failure);
        self.commands.push(record);
        failure.map_or(Ok(index), Err)
    }

    pub(super) fn capture(
        &mut self,
        argv: &[OsString],
        stage: &str,
        budget: Duration,
    ) -> Result<String, String> {
        let index = self.run(argv, stage, budget)?;
        fs::read_to_string(self.output.join(&self.commands[index].stdout))
            .map_err(|error| format!("read command output: {error}"))
    }
}
