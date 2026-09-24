// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Small process and output helpers shared by CLI integration tests.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

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

pub fn run(directory: &Path, args: &[&str]) -> Output {
    command()
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run ruyipack")
}

#[track_caller]
pub fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

#[track_caller]
pub fn quiet_success(output: &Output) {
    success(output);
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[track_caller]
pub fn rejected(output: &Output, message: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output_text(&output.stderr).contains(message), "{output:?}");
}

#[track_caller]
pub fn assert_file(path: impl AsRef<Path>, expected: &str) {
    let path = path.as_ref();
    let actual =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    assert_eq!(actual, expected, "{}", path.display());
}
