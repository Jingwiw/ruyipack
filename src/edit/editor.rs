// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! External editor invocation without a shell.

use std::{
    env,
    io::{self, IsTerminal},
    path::Path,
    process::{Command, Stdio},
};

pub(super) fn open(paths: &[&Path], explicit: Option<&str>) -> Result<(), String> {
    if explicit.is_none() && (!io::stdin().is_terminal() || !io::stderr().is_terminal()) {
        return Err(
            "editing requires a terminal; use --set, --prepare, or an explicit --editor command"
                .into(),
        );
    }
    let configured = explicit
        .map(str::to_owned)
        .or_else(|| env::var("VISUAL").ok().filter(|s| !s.trim().is_empty()))
        .or_else(|| env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vim".into());
    let words =
        shell_words::split(&configured).map_err(|e| format!("invalid editor command: {e}"))?;
    let (program, args) = words.split_first().ok_or("editor command is empty")?;
    let mut command = Command::new(program);
    command.args(args);
    if matches!(
        Path::new(program).file_name().and_then(|s| s.to_str()),
        Some("vi" | "vim" | "nvim")
    ) {
        // Draft text must not enable modelines, backups, or persistent undo.
        command.args([
            "-n",
            "-i",
            "NONE",
            "--cmd",
            "set nomodeline",
            "-c",
            "set nobackup nowritebackup noundofile",
            "--",
        ]);
    }
    // Keep stdout available for --diff and --stdout.
    let status = command
        .args(paths)
        .stdin(Stdio::inherit())
        .stdout(Stdio::from(io::stderr()))
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to start editor {program}: {e}"))?;
    if !status.success() {
        return Err(format!("editor exited with {status}"));
    }
    Ok(())
}
