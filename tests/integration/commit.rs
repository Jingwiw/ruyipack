// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Real local Git commits, conflicts and interrupted delivery; no remote services.

use super::support::{git, isolated_command, machine_report, recipe_workspace, success};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};
const SPEC: &str = include_str!("../fixtures/ed.spec");

fn run(root: &Path, args: &[&str]) -> Output {
    isolated_command(env!("CARGO_BIN_EXE_ruyipack"), root)
        .args(args)
        .output()
        .unwrap()
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let area = recipe_workspace(root.path(), "review", "ed", SPEC);
    let repo = root.path().join("openruyi");
    git(&repo, &["config", "user.name", "Package Author"]);
    git(&repo, &["config", "user.email", "package@example.org"]);
    git(&repo, &["config", "commit.gpgSign", "false"]);
    fs::write(repo.join("SPECS/ed/keep.patch"), "original patch\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "Package material"]);
    success(&run(
        root.path(),
        &["edit", "review", "--set=package.version=1.22.6", "--apply"],
    ));
    (root, repo, area)
}

#[test]
fn scoped_commit_preserves_repository_branch_and_retries_without_duplicate_history() {
    let (root, repo, area) = fixture();
    let recipe = area.join("recipe/SPECS/ed");
    fs::write(recipe.join("keep.patch"), "updated patch\n").unwrap();
    fs::write(recipe.join("new.patch"), "new patch\n").unwrap();
    fs::write(recipe.join("_constraints"), "<constraints/>\n").unwrap();
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    let preview = run(
        root.path(),
        &["commit", "review", "--dry-run", "--format=toml"],
    );
    success(&preview);
    let report = machine_report(&preview);
    assert_eq!(report["changes"].as_array().unwrap().len(), 3);
    assert!(report["diff"].as_str().unwrap().contains("+new patch"));
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    success(&run(root.path(), &["commit", "review", "--spec-only"]));
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s"]).stdout,
        b"SPECS: ed: Update 1.22.5 -> 1.22.6\n"
    );
    assert_eq!(
        fs::read(repo.join("SPECS/ed/keep.patch")).unwrap(),
        b"original patch\n"
    );
    let result = run(root.path(), &["commit", "review", "--format=toml"]);
    success(&result);
    assert_eq!(
        machine_report(&result)["changes"].as_array().unwrap().len(),
        2
    );
    let committed = git(&repo, &["rev-parse", "HEAD"]).stdout;
    success(&run(root.path(), &["commit", "review"]));
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, committed);
    assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
    assert_eq!(git(&repo, &["branch", "--show-current"]).stdout, b"main\n");
    fs::remove_file(recipe.join("keep.patch")).unwrap();
    success(&run(root.path(), &["commit", "review"]));
    assert!(!repo.join("SPECS/ed/keep.patch").exists());
    assert!(!repo.join("SPECS/ed/_constraints").exists());
    assert_eq!(
        fs::read_to_string(recipe.join("_constraints")).unwrap(),
        "<constraints/>\n"
    );
}

#[test]
fn dirty_repository_and_upstream_conflicts_do_not_apply_work() {
    let (root, repo, _) = fixture();
    fs::write(repo.join("notes"), "user notes").unwrap();
    let rejected = run(root.path(), &["commit", "review", "--format=toml"]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        machine_report(&rejected)["error"]
            .as_str()
            .unwrap()
            .contains("clean worktree")
    );
    assert_eq!(
        fs::read(repo.join("SPECS/ed/ed.spec")).unwrap(),
        SPEC.as_bytes()
    );
    fs::remove_file(repo.join("notes")).unwrap();
    fs::write(repo.join("SPECS/ed/ed.spec"), SPEC.replace("1.22.5", "2.0")).unwrap();
    git(&repo, &["commit", "-qam", "Another maintainer"]);
    let rejected = run(root.path(), &["commit", "review", "--format=toml"]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        machine_report(&rejected)["error"]
            .as_str()
            .unwrap()
            .contains("conflict")
    );
    assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn hook_failure_retains_exact_changes_and_retry_rejects_unrelated_edits() {
    use std::os::unix::fs::PermissionsExt;
    let (root, repo, area) = fixture();
    let hook = repo.join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        "#!/bin/sh\necho deliberate-hook-failure >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let failed = run(root.path(), &["commit", "review", "--format=toml"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(failed.stderr.is_empty());
    let report = machine_report(&failed);
    assert_eq!(report["retained"].as_bool(), Some(true));
    assert!(
        report["error"]
            .as_str()
            .unwrap()
            .contains("deliberate-hook-failure")
    );
    let preview = run(
        root.path(),
        &["commit", "review", "--dry-run", "--format=toml"],
    );
    success(&preview);
    let diff = machine_report(&preview)["diff"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(diff.contains("-Version:        1.22.5"));
    assert!(diff.contains("+Version:        1.22.6"));
    let pending = fs::read(area.join("commit.toml")).unwrap();
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    fs::write(repo.join("notes"), "don't commit this").unwrap();
    fs::remove_file(hook).unwrap();
    assert_eq!(
        run(root.path(), &["commit", "review"]).status.code(),
        Some(1)
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    fs::remove_file(repo.join("notes")).unwrap();
    success(&run(root.path(), &["commit", "review"]));
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    // Simulate loss of the final acknowledgement after Git committed successfully.
    fs::write(area.join("commit.toml"), pending).unwrap();
    success(&run(root.path(), &["commit", "review"]));
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    assert!(!area.join("commit.toml").exists());
}

#[test]
fn spec_only_import_keeps_unimported_materials_and_new_import_adds_all_files() {
    let (root, repo, _) = fixture();
    let input = root.path().join("input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("ed.spec"), SPEC).unwrap();
    fs::write(input.join("other.spec"), "unselected sibling").unwrap();
    success(&run(
        root.path(),
        &["new", "only", "--from-spec=input/ed.spec", "--pkgname=ed"],
    ));
    success(&run(
        root.path(),
        &["edit", "only", "--set=package.version=1.22.7", "--apply"],
    ));
    success(&run(root.path(), &["commit", "only"]));
    assert_eq!(
        fs::read(repo.join("SPECS/ed/keep.patch")).unwrap(),
        b"original patch\n"
    );
    success(&run(
        root.path(),
        &[
            "new",
            "other",
            "--from-dir=openruyi/SPECS/ed",
            "--pkgname=other",
        ],
    ));
    success(&run(root.path(), &["gen", "other", "--apply", "--offline"]));
    let preview = run(
        root.path(),
        &["commit", "other", "--dry-run", "--format=toml"],
    );
    success(&preview);
    let report = machine_report(&preview);
    assert_eq!(report["admission"]["allowed"].as_bool(), Some(true));
    assert_eq!(
        report["admission"]["spec_fallback"].as_str(),
        Some("ed.spec")
    );
    assert_eq!(
        report["admission"]["name_warning"]["directory"].as_str(),
        Some("other")
    );
    assert_eq!(
        report["admission"]["name_warning"]["literal"].as_str(),
        Some("ed")
    );
    let applied = run(
        root.path(),
        &["commit", "other", "-m", "other: initial package"],
    );
    success(&applied);
    assert!(
        String::from_utf8_lossy(&applied.stderr)
            .contains("[WARN] other: directory=other; SPEC Name=ed")
    );
    assert!(repo.join("SPECS/other/ed.spec").is_file());
    assert_eq!(
        fs::read(repo.join("SPECS/other/keep.patch")).unwrap(),
        b"original patch\n"
    );
}

#[test]
fn submit_failure_hidden_changes_and_detached_head_leave_inputs_unchanged() {
    let (root, repo, area) = fixture();
    let path = repo.join("SPECS/ed/ed.spec");
    for flag in ["assume-unchanged", "skip-worktree"] {
        git(
            &repo,
            &["update-index", &format!("--{flag}"), "SPECS/ed/ed.spec"],
        );
        fs::write(&path, "hidden user edit\n").unwrap();
        assert_eq!(
            run(root.path(), &["commit", "review"]).status.code(),
            Some(1)
        );
        assert_eq!(fs::read(&path).unwrap(), b"hidden user edit\n");
        git(
            &repo,
            &["update-index", &format!("--no-{flag}"), "SPECS/ed/ed.spec"],
        );
        git(&repo, &["restore", "SPECS/ed/ed.spec"]);
    }
    git(&repo, &["switch", "--detach", "--quiet"]);
    assert_eq!(
        run(root.path(), &["commit", "review"]).status.code(),
        Some(1)
    );
    git(&repo, &["switch", "main", "--quiet"]);
    let source = area.join("recipe/SPECS/ed/ed.spec");
    fs::write(&source, "Name: ed\n").unwrap();
    assert_eq!(
        run(root.path(), &["commit", "review"]).status.code(),
        Some(1)
    );
    assert_eq!(fs::read(path).unwrap(), SPEC.as_bytes());
    assert!(!area.join("commit.toml").exists());
}

#[test]
fn a_package_removed_upstream_is_not_recreated_by_a_later_commit() {
    let (root, repo, _) = fixture();
    git(&repo, &["rm", "-r", "SPECS/ed"]);
    git(&repo, &["commit", "-qm", "Remove package"]);
    assert_eq!(
        run(root.path(), &["commit", "review"]).status.code(),
        Some(1)
    );
    assert!(!repo.join("SPECS/ed").exists());
    assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn post_commit_timeout_reports_created_commit_and_retries_only_acknowledgement() {
    use std::os::unix::fs::PermissionsExt;
    let (root, repo, area) = fixture();
    let hook = repo.join(".git/hooks/post-commit");
    fs::write(&hook, "#!/bin/sh\nsleep 30\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let output = run(
        root.path(),
        &["commit", "review", "--timeout=1", "--format=toml"],
    );
    assert_eq!(output.status.code(), Some(1));
    let report = machine_report(&output);
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    assert_eq!(
        report["commit"].as_str().unwrap(),
        String::from_utf8_lossy(&head).trim()
    );
    assert!(area.join("commit.toml").exists());
    fs::remove_file(hook).unwrap();
    success(&run(root.path(), &["commit", "review"]));
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    assert!(!area.join("commit.toml").exists());
}

#[test]
fn material_admission_checks_the_delivery_not_the_work_directory() {
    let (root, repo, area) = fixture();
    let recipe = area.join("recipe/SPECS/ed");
    let path = recipe.join("ed.spec");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("BuildSystem:", "Patch0: new.patch\nBuildSystem:");
    fs::write(&path, &text).unwrap();
    let preview = run(
        root.path(),
        &["commit", "review", "--dry-run", "--format=toml"],
    );
    success(&preview);
    assert_eq!(
        machine_report(&preview)["admission"]["allowed"].as_bool(),
        Some(false)
    );
    assert!(
        machine_report(&preview)["diff"]
            .as_str()
            .unwrap()
            .contains("+Patch0:")
    );
    assert_eq!(
        run(root.path(), &["commit", "review"]).status.code(),
        Some(1)
    );
    fs::write(recipe.join("new.patch"), "patch bytes\n").unwrap();
    let rejected = run(
        root.path(),
        &["commit", "review", "--spec-only", "--format=toml"],
    );
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        machine_report(&rejected)["error"]
            .as_str()
            .unwrap()
            .contains("missing from the delivery")
    );
    assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
    success(&run(root.path(), &["commit", "review"]));
    assert_eq!(
        fs::read(repo.join("SPECS/ed/new.patch")).unwrap(),
        b"patch bytes\n"
    );
    // Unselected material exists in the repository; an unrelated WORK edit must not veto SPEC-only delivery.
    fs::write(recipe.join("new.patch"), "unfinished patch\n").unwrap();
    fs::write(&path, text + "\n# SPEC-only change\n").unwrap();
    success(&run(root.path(), &["commit", "review", "--spec-only"]));
    assert_eq!(
        fs::read(repo.join("SPECS/ed/new.patch")).unwrap(),
        b"patch bytes\n"
    );
}

#[cfg(unix)]
#[test]
fn preview_includes_empty_file_lifecycle_and_executable_modes() {
    use std::os::unix::fs::PermissionsExt;
    let (root, repo, area) = fixture();
    let recipe = area.join("recipe/SPECS/ed");
    let spec = recipe.join("ed.spec");
    fs::set_permissions(&spec, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(recipe.join("empty"), "").unwrap();
    let preview = run(
        root.path(),
        &["commit", "review", "--dry-run", "--format=toml"],
    );
    success(&preview);
    let report = machine_report(&preview);
    let diff = report["diff"].as_str().unwrap();
    assert!(diff.contains("old mode 100644\nnew mode 100755"));
    assert!(diff.contains("b/empty") && diff.contains("new file mode 100644"));
    success(&run(root.path(), &["commit", "review"]));
    assert!(
        git(&repo, &["show", "--summary", "HEAD"])
            .stdout
            .windows(11)
            .any(|s| s == b"mode change")
    );
    fs::remove_file(recipe.join("empty")).unwrap();
    let preview = run(
        root.path(),
        &["commit", "review", "--dry-run", "--format=toml"],
    );
    success(&preview);
    assert!(
        machine_report(&preview)["diff"]
            .as_str()
            .unwrap()
            .contains("deleted file mode 100644")
    );
    success(&run(root.path(), &["commit", "review"]));
    assert!(!repo.join("SPECS/ed/empty").exists());
}

#[test]
fn removed_source_reference_blocks_delivery_until_consumer_is_repaired() {
    let (root, repo, area) = fixture();
    let path = area.join("recipe/SPECS/ed/ed.spec");
    // Source0 has a valid remote digest in the fixture. Remove its declaration,
    // retaining an actual macro consumer rather than relying on file suffixes.
    let original = fs::read_to_string(&path).unwrap();
    let changed = original
        .lines()
        .filter(|line| !line.starts_with("Source0:") && !line.starts_with("#!RemoteAsset"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, format!("{changed}\n%prep\ncat %{{SOURCE0}}\n")).unwrap();
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    let failed = run(root.path(), &["commit", "review", "--format=toml"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        machine_report(&failed)["error"]
            .as_str()
            .unwrap()
            .contains("SOURCE0")
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
    fs::write(&path, format!("{changed}\n")).unwrap();
    success(&run(root.path(), &["commit", "review"]));
}

#[test]
fn excluded_tracked_files_require_separate_removal() {
    let (root, repo, _) = fixture();
    fs::write(repo.join("SPECS/ed/_constraints"), "<constraints/>").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "Add constraints"]);
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    let result = run(root.path(), &["commit", "review", "--format=toml"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        machine_report(&result)["error"]
            .as_str()
            .unwrap()
            .contains("excluded from Git publication")
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    assert_eq!(
        fs::read_to_string(repo.join("SPECS/ed/_constraints")).unwrap(),
        "<constraints/>"
    );
}

#[test]
fn required_build_blocks_unverified_publication_and_preserves_preview() {
    let (root, repo, _) = fixture();
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    for preview in [true, false] {
        let mut args = vec!["commit", "review", "--require-build", "--format=toml"];
        if preview {
            args.push("--dry-run");
        }
        let output = run(root.path(), &args);
        assert_eq!(output.status.code(), Some(1));
        let report = machine_report(&output);
        assert_eq!(report["admission"]["allowed"].as_bool(), Some(false));
        assert_eq!(report["build"]["status"].as_str(), Some("not-run"));
        if preview {
            assert!(report["diff"].as_str().unwrap().contains("1.22.6"));
        }
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
        assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
    }
    // Manual publication remains distinct from the opt-in local build gate.
    success(&run(root.path(), &["commit", "review"]));
}

#[test]
fn required_build_checks_stage_result_and_declared_inputs() {
    use sha2::{Digest, Sha256};
    let (root, repo, area) = fixture();
    let spec = area.join("recipe/SPECS/ed/ed.spec");
    let original = fs::read_to_string(&spec).unwrap();
    let hash = format!("{:x}", Sha256::digest(b"fixture source"));
    let candidate = original.replace(
        "56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc",
        &hash,
    );
    fs::write(&spec, &candidate).unwrap();
    let build = area.join("build");
    let mut inputs = vec![];
    for (name, bytes) in [
        ("SPECS/ed.spec", candidate.as_bytes()),
        ("SOURCES/ed-1.22.6.tar.lz", b"fixture source".as_slice()),
    ] {
        let path = build.join("input").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        inputs.push(serde_json::json!({"path":name,"executable":false,
            "size":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))}));
    }
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    for (stage, passed, changed, status, exit) in [
        ("prep", true, false, "passed", 1),
        ("build", false, false, "failed", 1),
        ("build", true, true, "stale", 1),
        ("build", true, false, "passed", 0),
    ] {
        fs::write(&spec, if changed { &original } else { &candidate }).unwrap();
        let receipt = serde_json::json!({"format_version":1,"package":"ed",
            "engine":"mock","stage":stage,"success":passed,"execution":{},"inputs":inputs});
        fs::write(
            build.join("receipt.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        let output = run(
            root.path(),
            &[
                "commit",
                "review",
                "--require-build",
                "--dry-run",
                "--format=toml",
            ],
        );
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert_eq!(
            machine_report(&output)["build"]["status"].as_str(),
            Some(status)
        );
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    }
    success(&run(root.path(), &["commit", "review", "--require-build"]));
    assert_eq!(
        fs::read(repo.join("SPECS/ed/ed.spec")).unwrap(),
        candidate.as_bytes()
    );
}

#[test]
fn task_commit_keeps_independent_commits_and_reports_partial_failure() {
    let (root, repo, _) = fixture();
    recipe_workspace(
        root.path(),
        "second",
        "other",
        &SPEC.replace("Name:           ed", "Name:           other"),
    );
    success(&run(
        root.path(),
        &["edit", "second", "--set=package.version=1.22.6", "--apply"],
    ));
    fs::write(
        root.path().join("plan.toml"),
        "[[packages]]\nwork='review'\n[[packages]]\nwork='missing'\n[[packages]]\nwork='second'\n",
    )
    .unwrap();
    let before = String::from_utf8(git(&repo, &["rev-parse", "HEAD"]).stdout).unwrap();
    let args = ["task", "--plan", "plan.toml", "commit", "--format=toml"];
    let result = run(root.path(), &args);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stderr.is_empty());
    let report = machine_report(&result);
    let tasks = report["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 3);
    for index in [0, 2] {
        assert_eq!(tasks[index]["success"].as_bool(), Some(true));
        assert!(tasks[index]["commit"].as_str().is_some());
    }
    assert_eq!(tasks[1]["success"].as_bool(), Some(false));
    assert!(tasks[1]["error"].as_str().unwrap().contains("missing"));
    assert_eq!(
        git(
            &repo,
            &["rev-list", "--count", &format!("{}..HEAD", before.trim())]
        )
        .stdout,
        b"2\n"
    );
    let head = git(&repo, &["rev-parse", "HEAD"]).stdout;
    let repeat = run(root.path(), &args);
    assert_eq!(repeat.status.code(), Some(1));
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, head);
    assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
}
