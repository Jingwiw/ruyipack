// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for field edits, persistent drafts and editor handoff.

mod drafts;
mod editor;
mod reports;
mod selection;
mod sources;

use super::support::{assert_file, success};

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

fn unchanged(directory: &Path) {
    assert_file(directory.join("ed.spec"), SPEC);
}

fn prepare(directory: &Path, names: &[&str], fields: &[&str]) -> PathBuf {
    let drafts = directory.join("drafts");
    let mut command = command(directory);
    command.args(names).arg("--prepare").arg(&drafts);
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
fn full_view_exposes_existing_fields_without_writing_files() {
    let directory = fixture(SPEC);
    let first = command(directory.path())
        .args(["ed.spec", "--all", "--view"])
        .output()
        .unwrap();
    success(&first);
    assert!(first.stderr.is_empty());
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&first.stdout).unwrap()).unwrap();
    let expected: toml::Table = toml::from_str(include_str!("../fixtures/ed.edit.toml")).unwrap();
    assert_eq!(document, expected);
    unchanged(directory.path());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn selected_view_and_schema_have_the_same_narrow_shape() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args(["ed.spec", "--view", "--field", "package.version"])
        .output()
        .unwrap();
    success(&output);
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(document.len(), 1);
    assert_eq!(document["package"].as_table().unwrap().len(), 1);
    let output = command(directory.path())
        .args(["ed.spec", "--schema", "--field", "package.version"])
        .output()
        .unwrap();
    success(&output);
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    let package = &schema["properties"]["package"];
    assert_eq!(package["additionalProperties"], false);
    assert_eq!(package["properties"].as_object().unwrap().len(), 1);
    assert_eq!(package["properties"]["version"]["type"], "string");
    assert!(
        package["properties"]["version"]["description"]
            .as_str()
            .unwrap()
            .contains("Version")
    );
    unchanged(directory.path());
}

#[test]
fn group_selection_keeps_descendants_without_duplicating_overlaps() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args([
            "ed.spec",
            "--view",
            "--field",
            "package.files",
            "--field",
            "package.files.doc",
        ])
        .output()
        .unwrap();
    success(&output);
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(document["package"].as_table().unwrap().len(), 1);
    let files = document["package"]["files"].as_table().unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        files.contains_key("doc") && files.contains_key("license") && files.contains_key("entries")
    );
    let bad = command(directory.path())
        .args(["ed.spec", "--view", "--field", "package.unknown"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(1), "{bad:?}");
    assert!(bad.stdout.is_empty());
}

#[test]
fn unchanged_assignment_preserves_every_source_byte() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.5", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, SPEC.as_bytes());
    let diff = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.5", "--diff"])
        .output()
        .unwrap();
    success(&diff);
    assert!(diff.stdout.is_empty());
    let saved = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.5"])
        .output()
        .unwrap();
    success(&saved);
    assert!(saved.stdout.is_empty());
    assert!(String::from_utf8_lossy(&saved.stderr).contains("Unchanged "));
    unchanged(directory.path());
}

#[test]
fn version_assignment_changes_only_the_original_value() {
    let directory = fixture(SPEC);
    let output = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.6", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, version_source("1.22.6").as_bytes());
    let diff = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.6", "--diff"])
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
        .args(["ed.spec", "--set", "package.version=1.22.6"])
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
            "ed.spec",
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
        command.arg("ed.spec");
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
            "ed.spec",
            "--set",
            "package.version=1.22.6",
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
        "ed.spec",
        "--set",
        "package.version=1.22.6",
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
fn noninteractive_edit_requires_an_explicit_input_mode() {
    let directory = fixture(SPEC);
    let output = command(directory.path()).arg("ed.spec").output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("select what to edit"));
    let output = command(directory.path())
        .args(["ed.spec", "--check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("select what to edit")
    );
    unchanged(directory.path());
}

#[test]
fn invalid_cli_combinations_fail_before_editing() {
    let directory = fixture(SPEC);
    // Keep real CLI wiring and assignment syntax here; the full conflict matrix
    // belongs to Cli::try_parse_from, without subprocess or filesystem setup.
    for args in [
        vec!["ed.spec", "--view", "--set", "package.version=2"],
        vec!["ed.spec", "--set", "package.version"],
    ] {
        let output = command(directory.path()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        unchanged(directory.path());
    }
}
