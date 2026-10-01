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
    let initialized = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
        .current_dir(directory.path())
        .arg("init")
        .output()
        .unwrap();
    assert!(initialized.status.success(), "{initialized:?}");
    let work = directory.path().join("work/review");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join(".config.toml"), "pkg = \"ed\"\n").unwrap();
    fs::write(work.join("ed.toml"), MANIFEST).expect("write WORK manifest");
    directory
}

fn gen_command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruyipack"));
    command
        .current_dir(directory)
        .args(["gen", "review", "--spec=ed.spec"]);
    command
}

#[test]
fn piped_human_diagnostics_never_emit_ansi_even_when_console_color_is_forced() {
    let directory = workspace();
    let path = directory.path().join("warning.spec");
    fs::write(&path, "%unknown value\n").unwrap();
    for (no_color, term) in [(false, "xterm"), (true, "xterm"), (false, "dumb")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ruyipack"));
        command
            .current_dir(directory.path())
            .args(["inspect", "--spec"])
            .arg(&path)
            .env("TERM", term)
            .env("CLICOLOR_FORCE", "1");
        if no_color {
            command.env("NO_COLOR", "1");
        } else {
            command.env_remove("NO_COLOR");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "[INFO] warning.spec: inspecting SPEC\n[WARN] spec[1:1] [rpmspec/W0002]: line not recognized\n"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn terminal_prefix_color_respects_no_color_and_dumb_term() {
    let directory = workspace();
    fs::write(directory.path().join("warning.spec"), "%unknown value\n").unwrap();
    for (no_color, term, colored) in [
        (false, "xterm", true),
        (true, "xterm", false),
        (false, "dumb", false),
    ] {
        let mut command = Command::new("script");
        command
            .current_dir(directory.path())
            .env("TERM", term)
            .env("CLICOLOR_FORCE", "1");
        if no_color {
            command.env("NO_COLOR", "1");
        } else {
            command.env_remove("NO_COLOR");
        }
        #[cfg(target_os = "macos")]
        command.args([
            "-q",
            "/dev/null",
            env!("CARGO_BIN_EXE_ruyipack"),
            "inspect",
            "--spec=warning.spec",
        ]);
        #[cfg(target_os = "linux")]
        command.args([
            "-q",
            "-e",
            "-c",
            &shell_words::join([
                env!("CARGO_BIN_EXE_ruyipack"),
                "inspect",
                "--spec=warning.spec",
            ]),
            "/dev/null",
        ]);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
        if colored {
            assert!(
                text.contains(
                    "\u{1b}[33m[WARN]\u{1b}[0m spec[1:1] [rpmspec/W0002]: line not recognized\n"
                ),
                "{text:?}"
            );
        } else {
            assert!(!text.contains('\u{1b}'), "{text:?}");
            assert!(
                text.contains("[WARN] spec[1:1] [rpmspec/W0002]: line not recognized\n"),
                "{text:?}"
            );
        }
        assert!(!output.stderr.contains(&0x1b), "{output:?}");
    }
}

fn closed_pipe() -> std::process::Stdio {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer.into()
}

#[test]
fn disconnected_stdout_returns_an_error_without_panicking() {
    let directory = workspace();
    fs::write(
        directory.path().join("ed.spec"),
        include_str!("fixtures/ed.spec"),
    )
    .unwrap();

    for args in [
        ["check", "--spec=ed.spec", "--format", "toml"].as_slice(),
        &["inspect", "--spec=ed.spec"],
        &["inspect", "--spec=ed.spec", "--format", "toml"],
        &["gen", "review", "--stdout"],
        &["inspect", "--spec=ed.spec", "--editable", "--all"],
        &[
            "edit",
            "--spec=ed.spec",
            "--all",
            "--check",
            "--format",
            "toml",
        ],
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--stdout",
        ],
        &[
            "edit",
            "--spec=ed.spec",
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
                    .starts_with("[INFO] ed.spec: stage saved:"),
                "{stderr}"
            );
            if args.contains(&"--stdout") {
                assert!(
                    lines
                        .next()
                        .unwrap()
                        .contains("review required after changing package.version:"),
                    "{stderr}"
                );
            }
        }
        assert!(
            lines
                .next()
                .unwrap()
                .starts_with("[ERROR] failed to write output to stdout:"),
            "{args:?}: {result:?}"
        );
        assert!(lines.next().is_none(), "{stderr}");
    }
}

#[test]
fn disconnected_stderr_and_both_streams_return_errors() {
    let directory = workspace();
    fs::write(directory.path().join("missing-tags.spec"), "Name: demo\n").unwrap();
    fs::write(directory.path().join("warning.spec"), "%unknown value\n").unwrap();
    for args in [
        ["check", "--spec=missing-tags.spec"].as_slice(),
        &["inspect", "--spec=warning.spec"],
        &["inspect", "--spec=absent.spec"],
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
}

#[test]
fn failed_publication_acknowledgement_does_not_allow_blind_retry() {
    let directory = workspace();
    fs::write(
        directory.path().join("ed.spec"),
        include_str!("fixtures/ed.spec"),
    )
    .unwrap();
    // Publishing succeeded even if its acknowledgement cannot reach the caller.
    // An exit code alone must not invite a blind replay against the old input.
    let original = fs::read(directory.path().join("ed.spec")).unwrap();
    assert_eq!(original, include_bytes!("fixtures/ed.spec"));
    let digest = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&original))
    };
    let result = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
        .current_dir(directory.path())
        .args([
            "edit",
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--apply",
            "--format",
            "toml",
        ])
        .stdout(closed_pipe())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("Files already written:"), "{stderr}");
    assert!(
        stderr.contains(
            fs::canonicalize(directory.path().join("ed.spec"))
                .unwrap()
                .to_str()
                .unwrap()
        ),
        "{stderr}"
    );
    let changed = fs::read(directory.path().join("ed.spec")).unwrap();
    assert_eq!(
        changed,
        String::from_utf8(original)
            .unwrap()
            .replace("1.22.5", "1.22.6")
            .as_bytes()
    );
    let retry = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
        .current_dir(directory.path())
        .args([
            "edit",
            "--spec=ed.spec",
            "--expect-sha256",
            &digest,
            "--set",
            "package.version=1.22.7",
            "--format",
            "toml",
        ])
        .output()
        .unwrap();
    assert_eq!(retry.status.code(), Some(1));
    let report: toml::Value = toml::from_str(&String::from_utf8_lossy(&retry.stdout)).unwrap();
    assert_eq!(report["error"]["code"].as_str(), Some("source-changed"));
    assert_eq!(fs::read(directory.path().join("ed.spec")).unwrap(), changed);
}

#[test]
fn failed_skip_acknowledgement_preserves_the_existing_target() {
    let directory = workspace();
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
