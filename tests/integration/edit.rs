// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for field edits, persistent drafts and editor handoff.

use super::support::{assert_file, success};

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const SOURCE: &str = include_str!("../fixtures/ed.spec");

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

fn unchanged(directory: &Path) {
    assert_file(directory.join("ed.spec"), SOURCE);
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

fn version_source(version: &str) -> String {
    let original = "Version:        1.22.5";
    assert_eq!(SOURCE.matches(original).count(), 1);
    SOURCE.replace(original, &format!("Version:        {version}"))
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
    let expected: toml::Table = toml::from_str(include_str!("../fixtures/ed.edit.toml")).unwrap();
    assert_eq!(document, expected);
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
    assert_eq!(bad.status.code(), Some(1), "{bad:?}");
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
    let directory = fixture();
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
    let expected = version_source("2.00").replace(
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
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
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
    assert_eq!(output.stdout, version_source("1.22.6").as_bytes());
    fs::write(
        &path,
        "[package]\nversion = '1.22.6'\nsummary = 'Outside selected fields'\n",
    )
    .unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("package.summary"));
    unchanged(directory.path());
}

#[test]
fn selected_draft_requires_every_selected_field() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    fs::write(drafts.join("ed.toml"), "[package]\n").unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .args(["--check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["format_version"], 2);
    assert_eq!(report["scope"], "selected-edit-static");
    assert_eq!(report["valid"], false);
    assert_eq!(report["files"][0]["valid"], false);
    assert_eq!(report["files"][0]["error"]["code"], "invalid-candidate");
    assert!(
        report["files"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("package.version")
    );
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
    assert_eq!(report["files"][0]["error"]["code"], "invalid-draft");
    let error = report["files"][0]["error"]["message"].as_str().unwrap();
    assert!(error.contains("ed.toml:2:"), "{error}");
    assert_file(path, "[package]\nversion = \"unterminated\n");
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
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    let report: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["files"][0]["valid"], true);
    assert_eq!(report["files"][1]["valid"], false);
    assert_eq!(report["files"][1]["error"]["code"], "invalid-candidate");
    let error = report["files"][1]["error"]["message"].as_str().unwrap();
    assert!(error.contains("package.version"), "{error}");
    assert!(error.contains("expected a string"), "{error}");
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    unchanged(directory.path());
    assert_file(directory.path().join("second.spec"), SOURCE);
}

#[test]
fn incomplete_batch_diagnostics_identify_each_candidate_without_publishing() {
    let directory = tempfile::tempdir().unwrap();
    let source = SOURCE.replace(
        "GPL-3.0-or-later AND LGPL-2.1-or-later",
        "%{package_license}",
    );
    assert_ne!(source, SOURCE);
    let names = ["first.spec", "second.spec"];
    for name in names {
        fs::write(directory.path().join(name), &source).unwrap();
    }
    let output = command(directory.path())
        .args(names)
        .args(["--set", "package.summary=Updated summary"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    for name in names {
        let path = fs::canonicalize(directory.path().join(name)).unwrap();
        let expected = format!(
            "{} (candidate): error: check incomplete because license expressions require RPM evaluation",
            path.display()
        );
        assert!(
            diagnostics.lines().any(|line| line == expected),
            "{diagnostics}"
        );
        assert_file(path, &source);
    }
}

#[test]
fn static_check_failure_blocks_even_forced_publication() {
    let directory = fixture();
    let source = SOURCE.replace("URL:            https://www.gnu.org/software/ed/\n", "");
    assert_ne!(source, SOURCE);
    let path = directory.path().join("ed.spec");
    fs::write(&path, &source).unwrap();
    let checked = command(directory.path())
        .args([
            "ed.spec",
            "--set",
            "package.version=1.22.6",
            "--check",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    assert!(checked.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert_eq!(report["files"][0]["valid"], false);
    assert_eq!(
        report["files"][0]["report"]["findings"][0]["code"],
        "RPM015"
    );
    assert_file(&path, &source);

    let target = directory.path().join("other.spec");
    fs::write(&target, "Keep this output\n").unwrap();
    for extra in [&[][..], &["--output", "other.spec", "--force"][..]] {
        let output = command(directory.path())
            .args(["ed.spec", "--set", "package.version=1.22.6"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("candidate failed static checks"));
        assert_file(&path, &source);
        assert_file(&target, "Keep this output\n");
    }
}

#[test]
fn prepared_dependency_edit_changes_only_its_source_value() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["build-requires"]);
    let path = drafts.join("ed.toml");
    let mut document: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let dependencies = document["build-requires"]["rpm"].as_array_mut().unwrap();
    assert_eq!(dependencies[4].as_str(), Some("lzip"));
    dependencies[4] = "xz".into();
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(drafts)
        .arg("--stdout")
        .output()
        .unwrap();
    success(&output);
    let expected = SOURCE.replace("BuildRequires:  lzip\n", "BuildRequires:  xz\n");
    assert_ne!(expected, SOURCE);
    assert_eq!(output.stdout, expected.as_bytes());
    unchanged(directory.path());
}

#[test]
fn saved_drafts_apply_without_reopening_an_editor() {
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
        .output()
        .unwrap();
    success(&output);
    for (name, version) in [("ed.spec", "1.22.6"), ("second.spec", "1.22.7")] {
        assert_file(directory.path().join(name), &(version_source(version)));
    }
}

#[test]
fn stale_prepared_source_is_not_overwritten() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    change_version(&drafts.join("ed.toml"), "1.22.6");
    let external = format!("{SOURCE}# Concurrent change\n");
    fs::write(directory.path().join("ed.spec"), &external).unwrap();
    let target = directory.path().join("other.spec");
    fs::write(&target, "Other file\n").unwrap();
    for extra in [&[][..], &["--output", "other.spec", "--force"][..]] {
        let output = command(directory.path())
            .arg("--from")
            .arg(&drafts)
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("changed"));
        assert_file(directory.path().join("ed.spec"), &external);
        assert_file(&target, "Other file\n");
    }
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
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["files"][0]["valid"], false);
    assert_eq!(report["files"][0]["error"]["code"], "source-changed");
    assert!(
        report["files"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("changed")
    );
    assert_eq!(report["files"][1]["valid"], true);
    assert_file(directory.path().join("ed.spec"), &external);
    assert_file(directory.path().join("second.spec"), SOURCE);
}

#[test]
fn explicit_output_cannot_overwrite_its_draft_or_saved_state() {
    let directory = fixture();
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    change_version(&drafts.join("ed.toml"), "1.22.6");
    let mut targets = vec![
        drafts.join("ed.toml"),
        drafts.join(".state/index.json"),
        drafts.join(".state/originals/0.spec"),
        drafts.join(".state/schema/0.json"),
    ];
    #[cfg(unix)]
    for (index, target) in targets.clone().iter().enumerate() {
        let alias = directory.path().join(format!("state-alias-{index}"));
        fs::hard_link(target, &alias).unwrap();
        targets.push(alias);
    }
    for target in targets {
        let before = fs::read(&target).unwrap();
        let output = command(directory.path())
            .arg("--from")
            .arg(&drafts)
            .arg("--force")
            .arg("--output")
            .arg(&target)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
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
    let directory = fixture();
    let output = command(directory.path()).arg("ed.spec").output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a terminal"));
    unchanged(directory.path());
}

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
    fs::write(directory.join("ed.spec"), SOURCE).unwrap();
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
        fs::write(directory.join("ed.spec"), SOURCE).unwrap();
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
    let directory = fixture();
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
        let directory = fixture();
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
        let directory = fixture();
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
        let expected = SOURCE.replace("A line-oriented text editor", "Updated summary");
        let in_place = args.is_empty();
        assert_file(
            directory.path().join("ed.spec"),
            if in_place { &expected } else { SOURCE },
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
    let directory = fixture();
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
        &(format!("{SOURCE}# Concurrent change\n")),
    );
}

#[cfg(unix)]
#[test]
fn editor_failure_retains_its_changed_draft() {
    let directory = fixture();
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

#[test]
fn invalid_cli_combinations_fail_before_editing() {
    let directory = fixture();
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

#[test]
fn check_json_versions_success_and_early_errors() {
    let directory = fixture();
    for (file, exit) in [("ed.spec", 0), ("missing.spec", 1)] {
        let output = command(directory.path())
            .args([
                file,
                "--field",
                "package.version",
                "--check",
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert!(output.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["format_version"], 2);
        assert_eq!(report["scope"], "selected-edit-static");
        assert_eq!(report["valid"], exit == 0);
        if exit == 0 {
            assert_eq!(
                report["files"][0]["report"]["evidence"]["stage"],
                "spec-static"
            );
        } else {
            assert!(report["files"].as_array().unwrap().is_empty());
            assert!(report["error"]["code"].is_string());
            assert!(report["error"]["message"].is_string());
        }
    }
    unchanged(directory.path());
}

#[test]
fn upgrade_review_is_visible_without_changing_static_check_success() {
    let directory = fixture();
    for (assignment, trigger) in [
        ("package.version=2", Some("package.version")),
        (
            "sources.0.url=https://example.org/new.tar.gz",
            Some("sources.0.url"),
        ),
        ("package.version=1.22.5", None),
        ("package.summary=Updated summary", None),
    ] {
        let output = command(directory.path())
            .args([
                "ed.spec", "--set", assignment, "--check", "--format", "json",
            ])
            .output()
            .unwrap();
        success(&output);
        assert!(output.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let file = &report["files"][0];
        assert_eq!(file["valid"], true);
        assert_eq!(file["report"]["evidence"]["status"], "pass");
        if let Some(trigger) = trigger {
            assert_eq!(file["review_triggers"], serde_json::json!([trigger]));
            assert_eq!(
                file["review_required"],
                serde_json::json!([
                    "source-content-and-digests",
                    "patch-applicability",
                    "native-build"
                ])
            );
        } else {
            assert_eq!(file["review_triggers"], serde_json::json!([]));
            assert_eq!(file["review_required"], serde_json::json!([]));
        }
        let preview = command(directory.path())
            .args(["ed.spec", "--set", assignment, "--stdout"])
            .output()
            .unwrap();
        success(&preview);
        assert_eq!(
            String::from_utf8_lossy(&preview.stderr).contains("review required"),
            trigger.is_some()
        );
        assert!(String::from_utf8_lossy(&preview.stdout).contains("sha256:56e107"));
    }
    unchanged(directory.path());
}

#[test]
fn edit_reports_bind_original_candidate_and_profile_without_inventing_a_path() {
    use sha2::{Digest, Sha256};
    let directory = fixture();
    let source_path = directory.path().join("ed.spec").canonicalize().unwrap();
    let output = command(directory.path())
        .args([
            "ed.spec",
            "--set",
            "package.version=2",
            "--check",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    success(&output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let file = &value["files"][0];
    assert_eq!(file["source"], source_path.to_str().unwrap());
    assert_eq!(
        file["report"]["input"]["display_path"],
        source_path.to_str().unwrap()
    );
    assert_eq!(
        file["original_sha256"],
        format!("{:x}", Sha256::digest(SOURCE.as_bytes()))
    );
    assert_eq!(
        file["report"]["input"]["sha256"],
        format!("{:x}", Sha256::digest(version_source("2").as_bytes()))
    );
    assert_eq!(file["report_subject"], "candidate");
    assert_eq!(file["profile"]["name"], "openruyi-v1");
    assert_eq!(
        file["profile"]["sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../../profiles/openruyi-v1/profile.toml"))
        )
    );
    assert_eq!(file["report"]["evidence"]["stage"], "spec-static");
    assert!(file.get("environment").is_none());
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
            let directory = fixture();
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
                        SOURCE.into()
                    }),
                );
            }
        }
    }
}
