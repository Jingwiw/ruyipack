// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Deletion preserves author work on refusal and never changes the recipe repository.

use super::support::{machine_report, recipe_workspace, run, success};
use std::fs;
use std::path::PathBuf;

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let area = recipe_workspace(
        root.path(),
        "review",
        "ed",
        include_str!("../fixtures/ed.spec"),
    );
    success(&run(root.path(), &["open", "review", "--editor=true"]));
    fs::write(area.join("ed.toml"), "# unfinished authoring\n").unwrap();
    (root, area)
}

#[test]
fn delete_previews_authoring_and_removes_only_the_confirmed_area() {
    let (root, area) = fixture();
    let repo = root.path().join("openruyi");
    let output = run(
        root.path(),
        &["delete", "review", "--dry-run", "--format=toml"],
    );
    success(&output);
    assert!(area.join("recipe/SPECS/ed/ed.spec").is_file());
    let report = machine_report(&output);
    assert!(
        report["authoring_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().ends_with("ed.toml"))
    );
    assert_eq!(
        run(root.path(), &["delete", "review"]).status.code(),
        Some(1)
    );
    let removed = run(
        root.path(),
        &["delete", "review", "--force", "--format=toml"],
    );
    success(&removed);
    assert!(removed.stderr.is_empty(), "{removed:?}");
    assert!(!area.exists());
    assert!(repo.join("SPECS/ed/ed.spec").is_file());
    assert_eq!(
        run(root.path(), &["delete", "review", "--force"])
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn failed_build_preflight_preserves_all_work_and_manual_recipe_removal_is_safe() {
    let (root, area) = fixture();
    let build = area.join("build");
    fs::create_dir(&build).unwrap();
    fs::write(build.join("receipt.json"), "invalid").unwrap();
    assert_eq!(
        run(root.path(), &["delete", "review", "--force"])
            .status
            .code(),
        Some(1)
    );
    assert!(area.join("recipe/SPECS/ed/ed.spec").is_file());
    fs::write(
        build.join("receipt.json"),
        r#"{"format_version":1,"backend":"compose","resources_retained":false}"#,
    )
    .unwrap();
    fs::remove_dir_all(area.join("recipe")).unwrap();
    success(&run(root.path(), &["delete", "review", "--force"]));
    assert!(!area.exists());
}
