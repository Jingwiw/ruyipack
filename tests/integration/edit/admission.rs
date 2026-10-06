// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

use super::{SPEC, assert_file, command, fixture, prepare, success};
use std::fs;

#[test]
fn authoring_can_save_a_local_change_without_claiming_whole_package_validity() {
    let source = SPEC.replace("https://www.gnu.org/software/ed/", "ftp://example.org/");
    let directory = fixture(&source);
    let checked = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set=package.version=1.22.6",
            "--check",
            "--format=toml",
        ])
        .output()
        .unwrap();
    success(&checked);
    let report = super::support::machine_report(&checked);
    assert_eq!(report["success"].as_bool(), Some(true));
    assert_eq!(report["valid"].as_bool(), Some(false));
    assert_eq!(report["files"][0]["admissible"].as_bool(), Some(true));
    assert_eq!(
        report["files"][0]["report"]["evidence"]["status"].as_str(),
        Some("fail")
    );
    assert_file(directory.path().join("ed.spec"), &source);

    let saved = command(directory.path())
        .args(["--spec=ed.spec", "--set=package.version=1.22.6", "--apply"])
        .output()
        .unwrap();
    success(&saved);
    assert_file(
        directory.path().join("ed.spec"),
        &source.replace("1.22.5", "1.22.6"),
    );

    // Saving did not waive URL policy for check or submission.
    for policy in ["authoring", "submit"] {
        let output = super::super::support::command()
            .args([
                "check",
                "--spec=ed.spec",
                "--policy",
                policy,
                "--format=toml",
            ])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let report = super::support::machine_report(&output);
        assert_eq!(report["valid"].as_bool(), Some(false));
        assert_eq!(report["findings"][0]["code"].as_str(), Some("RPK003"));
    }
}

#[test]
fn changed_invalid_inputs_block_but_dependency_advice_does_not() {
    let source = SPEC.replace("https://www.gnu.org/software/ed/", "ftp://example.org/old");
    let directory = fixture(&source);
    let output = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.url=ftp://example.org/new",
            "--check",
            "--format",
            "toml",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report = super::support::machine_report(&output);
    assert_eq!(report["success"].as_bool(), Some(false));
    assert_eq!(report["files"][0]["admissible"].as_bool(), Some(false));
    assert_eq!(
        report["files"][0]["introduced_static_blockers"].as_bool(),
        Some(true)
    );
    assert_file(directory.path().join("ed.spec"), &source);

    let source = ["autoconf", "automake", "libtool", "make"]
        .into_iter()
        .fold(SPEC.to_owned(), |text, tool| {
            text.replace(&format!("BuildRequires:  {tool}\n"), "")
        });
    let directory = fixture(&source);
    let drafts = prepare(directory.path(), &["ed.spec"], &["build-requires.rpm"]);
    fs::write(
        drafts.join("ed.toml"),
        "[build-requires]\nrpm = [\"zip\"]\n",
    )
    .unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(drafts)
        .args(["--check", "--format", "toml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let report = super::support::machine_report(&output);
    assert_eq!(report["files"][0]["admissible"].as_bool(), Some(true));
    assert_file(directory.path().join("ed.spec"), &source);
}

#[cfg(unix)]
#[test]
fn interactive_edit_reports_legacy_issues_and_digest_help_once() {
    let source = ["autoconf", "automake", "libtool", "make"].into_iter().fold(SPEC.to_owned(),
        |text, tool| text.replace(&format!("BuildRequires:  {tool}\n"), ""))
        .replace("#!RemoteAsset:  sha256:56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc", "#!RemoteAsset");
    let directory = fixture(&source);
    let editor = directory.path().join("editor.sh");
    fs::write(
        &editor,
        "sed 's/1.22.5/1.22.6/' \"$1\" > \"$1.next\"\nmv \"$1.next\" \"$1\"\n",
    )
    .unwrap();
    let output = command(directory.path())
        .args(["--spec=ed.spec", "--field", "package.version", "--editor"])
        .arg(format!(
            "/bin/sh {}",
            shell_words::quote(&editor.to_string_lossy())
        ))
        .arg("--apply")
        .output()
        .unwrap();
    success(&output);
    let log = String::from_utf8(output.stderr).unwrap();
    assert_eq!(log.matches(": candidate\n").count(), 1, "{log}");
    assert!(log.contains("autoconf, automake, libtool, make"), "{log}");
    assert_eq!(
        log.matches("[RPK004]: BuildRequires: consider explicitly declaring")
            .count(),
        1,
        "{log}"
    );
    for (code, text) in [("RPK004", "BuildSystem:"), ("RPK005", "Source0:")] {
        let line = source
            .lines()
            .position(|line| line.starts_with(text))
            .unwrap()
            + 1;
        assert!(
            log.contains(&format!("[WARN] spec[{line}:1] [{code}]")),
            "{log}"
        );
    }
    assert!(!log.contains(directory.path().to_str().unwrap()), "{log}");
    assert_eq!(log.matches("Source digests:").count(), 1, "{log}");
    assert_file(
        directory.path().join("ed.spec"),
        &source.replace("1.22.5", "1.22.6"),
    );
}
