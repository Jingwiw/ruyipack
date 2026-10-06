// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Real editor processes, recovery hints, and retained changes.

use super::{SPEC, assert_file, command, fixture, prepare, success, unchanged, version_source};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(unix)]
fn script(directory: &Path, body: &str) -> String {
    let path = directory.join("editor with spaces.sh");
    fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    format!("/bin/sh {}", shell_words::quote(&path.to_string_lossy()))
}

#[cfg(unix)]
#[test]
fn persistent_stage_paths_resume_from_a_changed_directory_with_shell_characters() {
    let root = tempfile::tempdir().unwrap();
    let run_shell = |shell: &str| {
        success(
            &Command::new("/bin/sh")
                .args(["-c", shell])
                .current_dir(root.path())
                .stdin(Stdio::null())
                .output()
                .unwrap(),
        );
    };
    let directory = root.path().join("author's $workspace");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("ed.spec"), SPEC).unwrap();
    let prepared = command(&directory)
        .args([
            "--spec=ed.spec",
            "--field",
            "package.version",
            "--prepare=-draft's $pending",
        ])
        .output()
        .unwrap();
    success(&prepared);
    let saved = directory.join("-draft's $pending");
    for action in ["--check", "--diff"] {
        let shell = format!(
            "{} edit --from={} {}",
            shell_words::quote(env!("CARGO_BIN_EXE_ruyipack")),
            shell_words::quote(&saved.to_string_lossy()),
            action
        );
        run_shell(&shell);
    }
    unchanged(&directory);
    for editor_fails in [false, true] {
        // Each case starts with a pristine source and a fresh persistent stage.
        fs::write(directory.join("ed.spec"), SPEC).unwrap();
        let pending = directory.join(".ruyipack-draft");
        if pending.exists() {
            fs::remove_dir_all(&pending).unwrap();
        }
        let editor = script(
            &directory,
            &format!(
                "sed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"\nexit {}",
                if editor_fails { 7 } else { 0 }
            ),
        );
        let output = command(&directory)
            .args([
                "--spec=ed.spec",
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
        let saved = directory.join(".ruyipack-draft/ed");
        let shell = format!(
            "{} edit --from={} --apply",
            shell_words::quote(env!("CARGO_BIN_EXE_ruyipack")),
            shell_words::quote(&saved.to_string_lossy())
        );
        run_shell(&shell);
        assert_file(directory.join("ed.spec"), &version_source("1.22.6"));
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
            "--spec=ed.spec",
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("saved:"));
    unchanged(directory.path());
}

#[cfg(unix)]
#[test]
fn explicit_apply_saves_checked_changes_and_keeps_the_persistent_stage() {
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
            edit.args(["--spec=ed.spec", "--field", "package.version"]);
        }
        let output = edit
            .args(["--editor", &editor, "--apply"])
            .output()
            .unwrap();
        success(&output);
        assert!(output.stdout.is_empty());
        assert_file(
            directory.path().join("ed.spec"),
            &(version_source("1.22.6")),
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("saved:"));
        assert!(!fs::read_dir(directory.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("ruyipack-edit-")
        }));
        assert_eq!(directory.path().join("drafts").exists(), persistent);
        assert!(
            directory
                .path()
                .join(if persistent {
                    "drafts/ed.toml"
                } else {
                    ".ruyipack-draft/ed/ed.toml"
                })
                .is_file()
        );
        if persistent {
            let draft = fs::read_to_string(directory.path().join("drafts/ed.toml")).unwrap();
            let document: toml::Table = toml::from_str(&draft).unwrap();
            assert_eq!(document["package"]["version"].as_str(), Some("1.22.6"));
            assert!(directory.path().join("drafts/.state/index.toml").is_file());
        }
    }
}

#[cfg(unix)]
#[test]
fn notification_failure_retains_editor_work_before_publication() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    for args in [
        &["--apply"][..],
        &["--stdout"][..],
        &["--apply", "--output", "copy.spec"][..],
    ] {
        let directory = fixture(SPEC);
        let editor = script(
            directory.path(),
            "sed 's/^summary = .*/summary = \"Updated summary\"/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"",
        );
        let (writer, reader) = UnixStream::pair().unwrap();
        drop(reader);
        let output = command(directory.path())
            .args([
                "--spec=ed.spec",
                "--field",
                "package.summary",
                "--editor",
                &editor,
            ])
            .args(args)
            .stderr(Stdio::from(OwnedFd::from(writer)))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        unchanged(directory.path());
        assert!(output.stdout.is_empty());
        assert!(!directory.path().join("copy.spec").exists());
        let document: toml::Table = toml::from_str(
            &fs::read_to_string(directory.path().join(".ruyipack-draft/ed/ed.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            document["package"]["summary"].as_str(),
            Some("Updated summary")
        );
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
        .args([
            "--spec=ed.spec",
            "--field",
            "package.version",
            "--editor",
            &editor,
            "--apply",
        ])
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
        .args([
            "--spec=ed.spec",
            "--field",
            "package.version",
            "--editor",
            &editor,
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("editor exited"));
    let retained = stderr
        .lines()
        .find_map(|line| line.strip_prefix("Edits retained: "))
        .unwrap();
    let draft = Path::new(retained).join("ed.toml");
    assert!(draft.is_file());
    assert!(fs::read_to_string(draft).unwrap().contains("1.22.6"));
    unchanged(directory.path());
}

#[cfg(unix)]
#[test]
fn editor_preview_and_copy_keep_all_persistent_stages() {
    for changed in [false, true] {
        for args in [
            vec!["--diff"],
            vec!["--stdout"],
            vec!["--apply", "--output", "copy.spec"],
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
                .args([
                    "--spec=ed.spec",
                    "--field",
                    "package.version",
                    "--editor",
                    &editor,
                ])
                .args(&args)
                .output()
                .unwrap();
            success(&output);
            let retained = directory.path().join(".ruyipack-draft/ed/ed.toml");
            let document: toml::Table =
                toml::from_str(&fs::read_to_string(retained).unwrap()).unwrap();
            assert_eq!(
                document["package"]["version"].as_str(),
                Some(if changed { "1.22.6" } else { "1.22.5" })
            );
            unchanged(directory.path());
            if args.contains(&"--output") {
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
