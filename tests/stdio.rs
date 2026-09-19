// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Closed-stream regressions isolated from concurrent command tests.

#![cfg(unix)]

use std::{fs, path::Path, process::Command};

const MANIFEST: &str = include_str!("../examples/ed/ed.toml");

fn workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("create workspace");
    fs::write(directory.path().join("ed.toml"), MANIFEST).expect("write manifest");
    directory
}

fn gen_command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruyipack"));
    command.current_dir(directory).args(["gen", "ed"]);
    command
}

#[test]
fn disconnected_standard_streams_return_errors_without_panicking() {
    use std::{io, process::Stdio};

    fn closed_pipe() -> Stdio {
        let (reader, writer) = io::pipe().unwrap();
        drop(reader);
        writer.into()
    }

    let directory = workspace();
    fs::write(
        directory.path().join("ed.spec"),
        include_str!("fixtures/ed.spec"),
    )
    .unwrap();
    fs::write(directory.path().join("missing-tags.spec"), "Name: demo\n").unwrap();
    fs::write(directory.path().join("warning.spec"), "%unknown value\n").unwrap();

    for args in [
        ["check", "ed.spec", "--format", "json"].as_slice(),
        &["inspect", "ed.spec"],
        &["inspect", "ed.spec", "--format", "json"],
        &["gen", "ed", "--stdout"],
        &["edit", "ed.spec", "--view"],
        &["edit", "ed.spec", "--check", "--format", "json"],
        &[
            "edit",
            "ed.spec",
            "--set",
            "package.version=1.22.6",
            "--stdout",
        ],
        &[
            "edit",
            "ed.spec",
            "--set",
            "package.version=1.22.6",
            "--diff",
        ],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
            .current_dir(directory.path())
            .args(args)
            .stdout(closed_pipe())
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{args:?}: {result:?}");
        let stderr = String::from_utf8_lossy(&result.stderr);
        let mut lines = stderr.lines();
        if args.contains(&"--set") {
            assert!(
                lines
                    .next()
                    .unwrap()
                    .contains("review required after changing package.version:"),
                "{stderr}"
            );
        }
        assert!(
            lines
                .next()
                .unwrap()
                .starts_with("error: failed to write output to stdout:"),
            "{args:?}: {result:?}"
        );
        assert!(lines.next().is_none(), "{stderr}");
    }

    for args in [
        ["check", "missing-tags.spec"].as_slice(),
        &["inspect", "warning.spec"],
        &["inspect", "absent.spec"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
            .current_dir(directory.path())
            .args(args)
            .stderr(closed_pipe())
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{args:?}: {result:?}");
        assert!(result.stdout.is_empty(), "{args:?}: {result:?}");
    }

    let result = gen_command(directory.path())
        .arg("--stdout")
        .stdout(closed_pipe())
        .stderr(closed_pipe())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1), "{result:?}");

    fs::write(directory.path().join("ed.spec"), "hand edited\n").unwrap();
    let result = gen_command(directory.path())
        .arg("--skip-existing")
        .stderr(closed_pipe())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        "hand edited\n"
    );
}
