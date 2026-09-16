// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Small process and output helpers shared by CLI integration tests.

use std::process::{Command, Output};

use serde_json::Value;

pub fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ruyipack"))
}

pub fn output_text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("command output is UTF-8")
}

/// Checks framing only; each caller owns status and stderr expectations.
pub fn json_line(output: &Output) -> Value {
    let stdout = output_text(&output.stdout);
    assert!(stdout.ends_with('\n'), "stdout has no trailing newline");
    assert_eq!(
        stdout.bytes().filter(|byte| *byte == b'\n').count(),
        1,
        "stdout is not one JSON line: {stdout}"
    );
    serde_json::from_slice(&output.stdout).expect("machine report is valid JSON")
}
