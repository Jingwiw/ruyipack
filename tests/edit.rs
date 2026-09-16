// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for field edits, persistent drafts and editor handoff.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const SOURCE: &str = include_str!("fixtures/ed.spec");

fn fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), SOURCE).unwrap();
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

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn unchanged(directory: &Path) {
    assert_eq!(
        fs::read_to_string(directory.join("ed.spec")).unwrap(),
        SOURCE
    );
}

fn prepare(directory: &Path, names: &[&str], fields: &[&str]) -> PathBuf {
    let drafts = directory.join("drafts");
    let mut command = command(directory);
    command.args(names).arg("--prepare").arg(&drafts);
    for field in fields {
        command.args(["--field", field]);
    }
    success(&command.output().unwrap());
    drafts
}

fn change_version(path: &Path, version: &str) {
    let mut document: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    document["package"]["version"] = version.into();
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
}

#[test]
fn full_view_exposes_existing_fields_without_writing_files() {
    let directory = fixture();
    let first = command(directory.path())
        .args(["ed.spec", "--view"])
        .output()
        .unwrap();
    success(&first);
    assert!(first.stderr.is_empty());
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&first.stdout).unwrap()).unwrap();
    assert_eq!(document["package"]["version"].as_str(), Some("1.22.5"));
    assert_eq!(document["spec"]["release"].as_str(), Some("%autorelease"));
    assert_eq!(document["build"]["system"].as_str(), Some("autotools"));
    assert_eq!(
        document["sources"]["0"]["sha256"].as_str().unwrap().len(),
        64
    );
    assert_eq!(
        document["build-requires"]["rpm"].as_array().unwrap().len(),
        5
    );
    assert!(
        document["package"]["files"]["entries"]
            .as_array()
            .unwrap()
            .contains(&"%{_bindir}/%{name}".into())
    );
    assert_eq!(
        first.stdout,
        command(directory.path())
            .args(["ed.spec", "--view"])
            .output()
            .unwrap()
            .stdout
    );
    unchanged(directory.path());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn selected_view_and_schema_have_the_same_narrow_shape() {
    let directory = fixture();
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
    let directory = fixture();
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
    assert!(!bad.status.success());
    assert!(bad.stdout.is_empty());
}

#[test]
fn unchanged_assignment_preserves_every_source_byte() {
    let directory = fixture();
    let output = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.5", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, SOURCE.as_bytes());
    let diff = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.5", "--diff"])
        .output()
        .unwrap();
    success(&diff);
    assert!(diff.stdout.is_empty());
    unchanged(directory.path());
}

#[test]
fn version_assignment_changes_only_the_original_value() {
    let directory = fixture();
    let output = command(directory.path())
        .args(["ed.spec", "--set", "package.version=1.22.6", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        output.stdout,
        SOURCE
            .replace("Version:        1.22.5", "Version:        1.22.6")
            .as_bytes()
    );
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
}

#[test]
fn repeated_set_options_preserve_equals_signs_and_string_types() {
    let directory = fixture();
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
    let expected = SOURCE
        .replace("Version:        1.22.5", "Version:        2.00")
        .replace(
            "URL:            https://www.gnu.org/software/ed/",
            "URL:            https://example.org/?a=b=c",
        );
    assert_eq!(output.stdout, expected.as_bytes());
    unchanged(directory.path());
}

#[test]
fn invalid_assignments_never_publish_a_partial_edit() {
    let directory = fixture();
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
        let output = command.arg("--force").output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        unchanged(directory.path());
    }
}

#[test]
fn prepared_drafts_round_trip_without_changes() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &[]);
    assert!(drafts.join("ed.toml").is_file());
    assert!(drafts.join(".state/index.json").is_file());
    assert!(drafts.join(".state/schema/0.json").is_file());
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .arg("--stdout")
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, SOURCE.as_bytes());
    unchanged(directory.path());
}

#[test]
fn selected_draft_merges_only_its_allowed_fields() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    let path = drafts.join("ed.toml");
    change_version(&path, "1.22.6");
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .arg("--stdout")
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        output.stdout,
        SOURCE
            .replace("Version:        1.22.5", "Version:        1.22.6")
            .as_bytes()
    );
    fs::write(
        &path,
        "[package]\nversion = '1.22.6'\nsummary = 'Outside selected fields'\n",
    )
    .unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .arg("--force")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("package.summary"));
    unchanged(directory.path());
}

#[test]
fn selected_draft_rejects_missing_and_wrongly_typed_fields() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    for content in ["[package]\n", "[package]\nversion = 2\n"] {
        fs::write(drafts.join("ed.toml"), content).unwrap();
        let output = command(directory.path())
            .arg("--from")
            .arg(&drafts)
            .args(["--check", "--format", "json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["valid"], false);
        assert_eq!(report["files"][0]["valid"], false);
        assert!(
            report["files"][0]["error"]
                .as_str()
                .unwrap()
                .contains("package.version")
        );
    }
    unchanged(directory.path());
}

#[test]
fn invalid_toml_reports_its_file_line_and_column_in_check_json() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    let path = drafts.join("ed.toml");
    fs::write(&path, "[package]\nversion = \"unterminated\n").unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .args(["--check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], false);
    let error = report["files"][0]["error"].as_str().unwrap();
    assert!(error.contains("ed.toml:2:"), "{error}");
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "[package]\nversion = \"unterminated\n"
    );
    unchanged(directory.path());
}

#[test]
fn all_prepared_files_are_checked_before_any_source_is_written() {
    let directory = fixture();
    fs::write(directory.path().join("second.spec"), SOURCE).unwrap();
    let drafts = prepare(
        directory.path(),
        &["ed.spec", "second.spec"],
        &["package.version"],
    );
    change_version(&drafts.join("ed.toml"), "1.22.6");
    fs::write(drafts.join("second.toml"), "[package]\nversion = 2\n").unwrap();
    let checked = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .args(["--check", "--format", "json"])
        .output()
        .unwrap();
    assert!(!checked.status.success());
    let report: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["files"][0]["valid"], true);
    assert_eq!(report["files"][1]["valid"], false);
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .arg("--force")
        .output()
        .unwrap();
    assert!(!output.status.success());
    unchanged(directory.path());
    assert_eq!(
        fs::read_to_string(directory.path().join("second.spec")).unwrap(),
        SOURCE
    );
}

#[test]
fn force_applies_each_valid_prepared_candidate() {
    let directory = fixture();
    fs::write(directory.path().join("second.spec"), SOURCE).unwrap();
    let drafts = prepare(
        directory.path(),
        &["ed.spec", "second.spec"],
        &["package.version"],
    );
    change_version(&drafts.join("ed.toml"), "1.22.6");
    change_version(&drafts.join("second.toml"), "1.22.7");
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .arg("--force")
        .output()
        .unwrap();
    success(&output);
    for (name, version) in [("ed.spec", "1.22.6"), ("second.spec", "1.22.7")] {
        assert_eq!(
            fs::read_to_string(directory.path().join(name)).unwrap(),
            SOURCE.replace(
                "Version:        1.22.5",
                &format!("Version:        {version}")
            )
        );
    }
}

#[test]
fn stale_prepared_source_is_not_overwritten_even_with_force() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    change_version(&drafts.join("ed.toml"), "1.22.6");
    let external = format!("{SOURCE}# Concurrent change\n");
    fs::write(directory.path().join("ed.spec"), &external).unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .arg("--force")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed"));
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        external
    );
}

#[test]
fn stale_draft_does_not_hide_other_files_check_results() {
    let directory = fixture();
    fs::write(directory.path().join("second.spec"), SOURCE).unwrap();
    let drafts = prepare(
        directory.path(),
        &["ed.spec", "second.spec"],
        &["package.version"],
    );
    let external = format!("{SOURCE}# Concurrent change\n");
    fs::write(directory.path().join("ed.spec"), &external).unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .args(["--check", "--format", "json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["files"][0]["valid"], false);
    assert!(
        report["files"][0]["error"]
            .as_str()
            .unwrap()
            .contains("changed")
    );
    assert_eq!(report["files"][1]["valid"], true);
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        external
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("second.spec")).unwrap(),
        SOURCE
    );
}

#[test]
fn explicit_output_cannot_overwrite_its_draft_or_saved_state() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    change_version(&drafts.join("ed.toml"), "1.22.6");
    for target in [
        drafts.join("ed.toml"),
        drafts.join(".state/index.json"),
        drafts.join(".state/originals/0.spec"),
    ] {
        let before = fs::read(&target).unwrap();
        let output = command(directory.path())
            .arg("--from")
            .arg(&drafts)
            .arg("--force")
            .arg("--output")
            .arg(&target)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert_eq!(fs::read(target).unwrap(), before);
        unchanged(directory.path());
    }
}

#[cfg(unix)]
#[test]
fn draft_files_and_saved_originals_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &[]);
    for path in [
        "ed.toml",
        ".state/index.json",
        ".state/originals/0.spec",
        ".state/schema/0.json",
    ] {
        let permissions = fs::metadata(drafts.join(path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(permissions & 0o077, 0, "{path}: {permissions:o}");
    }
}

#[test]
fn explicit_output_writes_a_copy_without_changing_the_source() {
    let directory = fixture();
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
    assert_eq!(
        fs::read_to_string(directory.path().join("edited.spec")).unwrap(),
        SOURCE.replace("Version:        1.22.5", "Version:        1.22.6")
    );
    unchanged(directory.path());
}

#[test]
fn invalid_cli_combinations_fail_before_editing() {
    let directory = fixture();
    for args in [
        vec!["ed.spec", "--view", "--set", "package.version=2"],
        vec!["ed.spec", "--schema", "--force"],
        vec!["ed.spec", "--set", "package.version"],
        vec![
            "ed.spec",
            "--set",
            "package.version=2",
            "--diff",
            "--stdout",
        ],
    ] {
        let output = command(directory.path()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        unchanged(directory.path());
    }
}
