// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Trusted editor commands: workspace override, then Git's editor selection.

use std::{
    io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

pub(crate) fn open(
    paths: &[&Path],
    explicit: Option<&str>,
    repository: &Path,
) -> Result<(), String> {
    let workspace = crate::workspace::discover_optional().map_err(|error| error.to_string())?;
    let configured = if let Some(editor) = explicit.or_else(|| {
        workspace
            .as_ref()
            .and_then(crate::workspace::Workspace::editor)
    }) {
        editor.to_owned()
    } else {
        let output = crate::host_process::capture(
            Command::new("git")
                .current_dir(
                    workspace
                        .as_ref()
                        .map(crate::workspace::Workspace::recipes)
                        .filter(|path| path.is_dir())
                        .unwrap_or(repository),
                )
                .args(["var", "GIT_EDITOR"]),
            Duration::from_secs(30),
            64 * 1024,
        )
        .map_err(|error| format!("cannot select Git editor: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "cannot select Git editor: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        String::from_utf8(output.stdout).map_err(|error| error.to_string())?
    };
    let configured = configured.trim();
    if configured.is_empty() {
        return Err("editor command is empty".into());
    }
    // Git editor values are trusted shell commands. Paths remain separate arguments,
    // never interpolated into the command; stdout stays available for SPEC/diff data.
    let status = Command::new("/bin/sh")
        .args(["-c", &format!("{configured} \"$@\""), configured])
        .args(paths)
        .stdin(Stdio::inherit())
        .stdout(Stdio::from(io::stderr()))
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to start editor: {error}"))?;
    if !status.success() {
        return Err(format!("editor exited with {status}"));
    }
    Ok(())
}
