// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Real editor processes, recovery hints, and retained changes.

use super::{SPEC, assert_file, command, fixture, prepare, success, unchanged, version_source};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[cfg(unix)]
fn script(directory: &Path, body: &str) -> String {
    let path = directory.join("editor with spaces.sh");
    fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    format!("/bin/sh {}", shell_words::quote(&path.to_string_lossy()))
}

#[cfg(unix)]
#[test]
fn draft_hints_handle_hyphen_paths_shell_characters_and_a_changed_directory() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("author's $workspace");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("ed.spec"), SPEC).unwrap();
    let execute_hint = |output: &Output, label: &str| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let hint = stderr
            .lines()
            .find_map(|line| line.strip_prefix(label))
            .unwrap();
        let bin = Path::new(env!("CARGO_BIN_EXE_ruyipack")).parent().unwrap();
        let mut paths = vec![bin.to_path_buf()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let result = Command::new("/bin/sh")
            .args(["-c", hint])
            .current_dir(root.path())
            .env("PATH", std::env::join_paths(paths).unwrap())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        success(&result);
    };
    let prepared = command(&directory)
        .args([
            "ed.spec",
            "--field",
            "package.version",
            "--prepare=-draft's $pending",
        ])
        .output()
        .unwrap();
    success(&prepared);
    execute_hint(&prepared, "Check: ");
    execute_hint(&prepared, "Preview: ");
    unchanged(&directory);

    for editor_fails in [false, true] {
        fs::write(directory.join("ed.spec"), SPEC).unwrap();
        let editor = script(
            &directory,
            &format!(
                "sed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"\nexit {}",
                if editor_fails { 7 } else { 0 }
            ),
        );
        let output = command(&directory)
            .args([
                "ed.spec",
                "--field",
                "package.version",
                "--editor",
                &editor,
                "--stdout",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(i32::from(editor_fails)));
        unchanged(&directory);
        execute_hint(&output, "Resume: ");
        assert_file(directory.join("ed.spec"), &(version_source("1.22.6")));
    }
}

#[cfg(unix)]
#[test]
fn quoted_editor_command_edits_toml_without_polluting_spec_stdout() {
    let directory = fixture(SPEC);
    let editor = script(
        directory.path(),
        "printf 'EDITOR_OUTPUT\\n'\nsed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"",
    );
    let output = command(directory.path())
        .args([
            "ed.spec",
            "--field",
            "package.version",
            "--editor",
            &editor,
            "--stdout",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, version_source("1.22.6").as_bytes());
    assert!(String::from_utf8_lossy(&output.stderr).contains("EDITOR_OUTPUT"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Drafts retained:"));
    unchanged(directory.path());
}

#[cfg(unix)]
#[test]
fn successful_editor_saves_checked_changes_and_cleans_temporary_drafts() {
    for persistent in [false, true] {
        let directory = fixture(SPEC);
        let editor = script(
            directory.path(),
            "sed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"",
        );
        let mut edit = command(directory.path());
        if persistent {
            let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
            edit.arg("--from").arg(drafts);
        } else {
            edit.args(["ed.spec", "--field", "package.version"]);
        }
        let output = edit.args(["--editor", &editor]).output().unwrap();
        success(&output);
        assert!(output.stdout.is_empty());
        assert_file(
            directory.path().join("ed.spec"),
            &(version_source("1.22.6")),
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Drafts retained:"));
        assert!(!fs::read_dir(directory.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("ruyipack-edit-")
        }));
        assert_eq!(directory.path().join("drafts").exists(), persistent);
        if persistent {
            let draft = fs::read_to_string(directory.path().join("drafts/ed.toml")).unwrap();
            let document: toml::Table = toml::from_str(&draft).unwrap();
            assert_eq!(document["package"]["version"].as_str(), Some("1.22.6"));
            assert!(directory.path().join("drafts/.state/index.json").is_file());
        }
    }
}

#[cfg(unix)]
#[test]
fn notification_failure_keeps_only_unapplied_editor_changes() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};

    for args in [&[][..], &["--stdout"][..], &["--output", "copy.spec"][..]] {
        let directory = fixture(SPEC);
        let editor = script(
            directory.path(),
            "sed 's/^summary = .*/summary = \"Updated summary\"/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"",
        );
        let (writer, reader) = UnixStream::pair().unwrap();
        drop(reader);
        let output = command(directory.path())
            .args(["ed.spec", "--field", "package.summary", "--editor", &editor])
            .args(args)
            .stderr(Stdio::from(OwnedFd::from(writer)))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        let expected = SPEC.replace("A line-oriented text editor", "Updated summary");
        let in_place = args.is_empty();
        assert_file(
            directory.path().join("ed.spec"),
            if in_place { &expected } else { SPEC },
        );
        if args == ["--stdout"] {
            assert_eq!(output.stdout, expected.as_bytes());
        } else if !in_place {
            assert_file(directory.path().join("copy.spec"), &expected);
        }
        let retained = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("ruyipack-edit-")
            })
            .collect::<Vec<_>>();
        assert_eq!(retained.len(), usize::from(!in_place), "{args:?}");
        for draft in retained {
            let document: toml::Table =
                toml::from_str(&fs::read_to_string(draft.join("ed.toml")).unwrap()).unwrap();
            assert_eq!(
                document["package"]["summary"].as_str(),
                Some("Updated summary")
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn source_changed_while_editor_runs_is_not_overwritten() {
    let directory = fixture(SPEC);
    let editor = script(
        directory.path(),
        "printf '# Concurrent change\\n' >> ed.spec\nsed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"",
    );
    let output = command(directory.path())
        .args(["ed.spec", "--field", "package.version", "--editor", &editor])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("source changed"));
    assert_file(
        directory.path().join("ed.spec"),
        &(format!("{SPEC}# Concurrent change\n")),
    );
}

#[cfg(unix)]
#[test]
fn editor_failure_retains_its_changed_draft() {
    let directory = fixture(SPEC);
    let editor = script(
        directory.path(),
        "sed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"\nexit 7",
    );
    let output = command(directory.path())
        .args(["ed.spec", "--field", "package.version", "--editor", &editor])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("editor exited"));
    let retained = stderr
        .lines()
        .find_map(|line| line.strip_prefix("Drafts retained: "))
        .unwrap();
    let draft = Path::new(retained).join("ed.toml");
    assert!(draft.is_file());
    assert!(fs::read_to_string(draft).unwrap().contains("1.22.6"));
    unchanged(directory.path());
}

#[cfg(unix)]
#[test]
fn editor_preview_and_copy_retain_only_actual_candidate_changes() {
    for changed in [false, true] {
        for args in [
            vec!["--diff"],
            vec!["--stdout"],
            vec!["--output", "copy.spec"],
        ] {
            let directory = fixture(SPEC);
            let editor = script(
                directory.path(),
                if changed {
                    "sed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\""
                } else {
                    "true"
                },
            );
            let output = command(directory.path())
                .args(["ed.spec", "--field", "package.version", "--editor", &editor])
                .args(&args)
                .output()
                .unwrap();
            success(&output);
            let retained = String::from_utf8_lossy(&output.stderr)
                .lines()
                .find_map(|line| line.strip_prefix("Drafts retained: ").map(PathBuf::from));
            assert_eq!(retained.is_some(), changed, "{args:?}: {output:?}");
            if let Some(retained) = retained {
                assert!(
                    fs::read_to_string(retained.join("ed.toml"))
                        .unwrap()
                        .contains("1.22.6")
                );
            }
            unchanged(directory.path());
            if args[0] == "--output" {
                assert_file(
                    directory.path().join("copy.spec"),
                    &(if changed {
                        version_source("1.22.6")
                    } else {
                        SPEC.into()
                    }),
                );
            }
        }
    }
}
