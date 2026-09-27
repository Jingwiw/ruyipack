// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Saved-draft shape, identity, privacy, and publication protection.

use super::{
    SPEC, assert_file, change_version, command, fixture, prepare, success, unchanged,
    version_source,
};
use std::fs;

#[test]
fn prepared_drafts_round_trip_without_changes() {
    let directory = fixture(SPEC);
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
    assert_eq!(output.stdout, SPEC.as_bytes());
    unchanged(directory.path());
}

#[test]
fn selected_draft_changes_only_its_selected_fields() {
    let directory = fixture(SPEC);
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
    let directory = fixture(SPEC);
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
fn all_prepared_files_are_checked_before_any_source_is_written() {
    let directory = fixture(SPEC);
    fs::write(directory.path().join("second.spec"), SPEC).unwrap();
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
    assert_file(directory.path().join("second.spec"), SPEC);
}

#[test]
fn prepared_dependency_edit_changes_only_its_source_value() {
    let directory = fixture(SPEC);
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
    let expected = SPEC.replace("BuildRequires:  lzip\n", "BuildRequires:  xz\n");
    assert_ne!(expected, SPEC);
    assert_eq!(output.stdout, expected.as_bytes());
    unchanged(directory.path());
}

#[test]
fn saved_drafts_apply_without_reopening_an_editor() {
    let directory = fixture(SPEC);
    fs::write(directory.path().join("second.spec"), SPEC).unwrap();
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
    let directory = fixture(SPEC);
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    change_version(&drafts.join("ed.toml"), "1.22.6");
    let external = format!("{SPEC}# Concurrent change\n");
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
    let directory = fixture(SPEC);
    fs::write(directory.path().join("second.spec"), SPEC).unwrap();
    let drafts = prepare(
        directory.path(),
        &["ed.spec", "second.spec"],
        &["package.version"],
    );
    let external = format!("{SPEC}# Concurrent change\n");
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
    assert_file(directory.path().join("second.spec"), SPEC);
}

#[test]
fn explicit_output_cannot_overwrite_its_draft_or_saved_state() {
    let directory = fixture(SPEC);
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

    let directory = fixture(SPEC);
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
