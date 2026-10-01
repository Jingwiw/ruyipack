// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for field edits, persistent drafts and editor handoff.

mod admission;
mod drafts;
mod editor;
mod reports;
mod selection;
mod sources;
mod stage_model;
mod terminal;

use super::support::{self, assert_file, success};

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const SPEC: &str = include_str!("../fixtures/ed.spec");

fn fixture(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), source).unwrap();
    directory
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruyipack"));
    command
        .arg("edit")
        .current_dir(directory)
        .env("TMPDIR", directory)
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .stdin(Stdio::null());
    command
}

fn resume(directory: &Path, drafts: &Path) -> Command {
    let mut command = command(directory);
    command.arg("--from").arg(drafts);
    command
}

#[track_caller]
fn preview(directory: &Path, assignment: &str, expected: &str) -> std::process::Output {
    let output = command(directory)
        .args(["--spec=ed.spec", "--set", assignment, "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, expected.as_bytes());
    output
}

fn inspect(directory: &Path) -> Command {
    let mut command = super::support::command();
    command
        .args(["inspect", "--editable"])
        .current_dir(directory)
        .stdin(Stdio::null());
    command
}

fn unchanged(directory: &Path) {
    assert_file(directory.join("ed.spec"), SPEC);
}

fn prepare(directory: &Path, names: &[&str], fields: &[&str]) -> PathBuf {
    let drafts = directory.join("drafts");
    let mut command = command(directory);
    command
        .args(names.iter().copied().flat_map(|name| ["--spec", name]))
        .arg("--prepare")
        .arg(&drafts);
    if fields.is_empty() {
        command.arg("--all");
    }
    for field in fields {
        command.args(["--field", field]);
    }
    success(&command.output().unwrap());
    drafts
}

fn version_source(version: &str) -> String {
    let original = "Version:        1.22.5";
    assert_eq!(SPEC.matches(original).count(), 1);
    SPEC.replace(original, &format!("Version:        {version}"))
}

fn change_version(path: &Path, version: &str) {
    let mut document: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    document["package"]["version"] = version.into();
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

#[test]
fn unchanged_assignment_preserves_every_source_byte() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.5",
            "--stdout",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, SPEC.as_bytes());
    let diff = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.5",
            "--diff",
        ])
        .output()
        .unwrap();
    success(&diff);
    assert!(diff.stdout.is_empty());
    let saved = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.5",
            "--apply",
        ])
        .output()
        .unwrap();
    success(&saved);
    assert!(saved.stdout.is_empty());
    assert!(String::from_utf8_lossy(&saved.stderr).contains("Unchanged"));
    unchanged(directory.path());
}

#[test]
fn version_assignment_changes_only_the_original_value() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--stdout",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, version_source("1.22.6").as_bytes());
    let diff = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--diff",
        ])
        .output()
        .unwrap();
    success(&diff);
    let diff = std::str::from_utf8(&diff.stdout).unwrap();
    assert!(diff.contains("-Version:        1.22.5\n+Version:        1.22.6\n"));
    assert_eq!(
        diff.lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count(),
        1
    );
    unchanged(directory.path());
    let saved = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--apply",
        ])
        .output()
        .unwrap();
    success(&saved);
    assert!(saved.stdout.is_empty());
    assert_file(
        directory.path().join("ed.spec"),
        &(version_source("1.22.6")),
    );
}

#[test]
fn repeated_set_options_preserve_equals_signs_and_string_types() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=2.00",
            "--set",
            "package.url=https://example.org/?a=b=c",
            "--stdout",
        ])
        .output()
        .unwrap();
    success(&output);
    let expected = version_source("2.00").replace(
        "URL:            https://www.gnu.org/software/ed/",
        "URL:            https://example.org/?a=b=c",
    );
    assert_eq!(output.stdout, expected.as_bytes());
    unchanged(directory.path());
}

#[test]
fn invalid_assignments_never_publish_a_partial_edit() {
    let directory = fixture(SPEC);
    for assignments in [
        vec!["package.version=1.22.6", "package.version=1.22.7"],
        vec!["package.version=1.22.6", "package.unknown=present"],
        vec!["build-requires.rpm=lzip"],
        vec!["package.version=2\nName: injected"],
        vec!["package.version=2", "package.summary="],
    ] {
        let mut command = command(directory.path());
        command.args(["--spec=ed.spec", "--apply"]);
        for assignment in assignments {
            command.args(["--set", assignment]);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        unchanged(directory.path());
    }
}

#[test]
fn explicit_output_writes_a_copy_without_changing_the_source() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--apply",
            "-o",
            "edited.spec",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_file(
        directory.path().join("edited.spec"),
        &(version_source("1.22.6")),
    );
    unchanged(directory.path());
    let target = directory.path().join("edited.spec");
    fs::write(&target, "Other file\n").unwrap();
    let args = [
        "--spec=ed.spec",
        "--set",
        "package.version=1.22.6",
        "--apply",
        "--output",
        "edited.spec",
    ];
    let refused = command(directory.path()).args(args).output().unwrap();
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--force"));
    assert_file(&target, "Other file\n");
    unchanged(directory.path());
    let forced = command(directory.path())
        .args(args)
        .arg("--force")
        .output()
        .unwrap();
    success(&forced);
    assert_file(&target, &(version_source("1.22.6")));
    unchanged(directory.path());
}

#[test]
fn default_noninteractive_edit_retains_a_stage_and_check_is_explicit() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args(["--spec=ed.spec"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("editing requires a terminal"));
    assert!(
        directory
            .path()
            .join(".ruyipack-stage/ed/ed.toml")
            .is_file()
    );
    let output = command(directory.path())
        .args(["--spec=ed.spec", "--check", "--format", "toml"])
        .output()
        .unwrap();
    success(&output);
    let report = super::support::machine_report(&output);
    assert_eq!(report["valid"].as_bool(), Some(true));
    assert_eq!(report["files"][0]["state"].as_str(), Some("candidate"));
    unchanged(directory.path());
}

#[test]
fn invalid_cli_combinations_fail_before_editing() {
    let directory = fixture(SPEC);
    // Keep real CLI wiring and assignment syntax here; the full conflict matrix
    // belongs to Cli::try_parse_from, without subprocess or filesystem setup.
    for args in [
        vec![
            "--spec=ed.spec",
            "--field",
            "package.version",
            "--set",
            "package.version=2",
        ],
        vec!["--spec=ed.spec", "--set", "package.version"],
    ] {
        let output = command(directory.path()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        unchanged(directory.path());
    }
}
