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

#[test]
fn selective_cleanup_keeps_recipe_and_plan_deletion_reports_each_result() {
    let (root, area) = fixture();
    let build = area.join("build");
    fs::create_dir(&build).unwrap();
    fs::write(build.join("receipt.json"), "invalid local build").unwrap();
    let remote = run(
        root.path(),
        &["clean", "review", "--remote", "--force", "--format=toml"],
    );
    success(&remote);
    assert!(remote.stderr.is_empty());
    assert_eq!(machine_report(&remote)["success"].as_bool(), Some(true));
    assert!(area.join("ed.toml").is_file());
    assert_eq!(
        fs::read_to_string(build.join("receipt.json")).unwrap(),
        "invalid local build"
    );
    fs::write(
        build.join("receipt.json"),
        r#"{"format_version":1,"backend":"compose","resources_retained":false}"#,
    )
    .unwrap();
    success(&run(
        root.path(),
        &["clean", "review", "--force", "--format=toml"],
    ));
    assert!(!build.exists());
    assert!(area.join("ed.toml").exists());
    fs::write(
        root.path().join("plan.toml"),
        "[[packages]]\nwork='missing'\n[[packages]]\nwork='review'\n",
    )
    .unwrap();
    let output = run(
        root.path(),
        &[
            "task",
            "--plan=plan.toml",
            "delete",
            "--force",
            "--format=toml",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report = machine_report(&output);
    assert_eq!(report["results"][0]["success"].as_bool(), Some(false));
    assert_eq!(report["results"][1]["success"].as_bool(), Some(true));
    assert!(!area.exists());
    assert!(root.path().join("openruyi/SPECS/ed/ed.spec").exists());
}
