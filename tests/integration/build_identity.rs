// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Exercise Cargo invalidation, not merely the Git query that supplies identity.

use super::support::{command, json_line, success};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Identity Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    success(&output);
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn build(root: &Path, target: &Path, supplied: Option<(&str, &str)>) -> String {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(root)
        .args(["run", "--offline", "--quiet"])
        .env("CARGO_TARGET_DIR", target)
        .env_remove("RUYIPACK_SOURCE_REVISION")
        .env_remove("RUYIPACK_SOURCE_DIRTY");
    if let Some((revision, dirty)) = supplied {
        command
            .env("RUYIPACK_SOURCE_REVISION", revision)
            .env("RUYIPACK_SOURCE_DIRTY", dirty);
    }
    let output = command.output().unwrap();
    success(&output);
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn cargo_refreshes_identity_for_commits_dirty_files_worktrees_and_archives() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("repo");
    let target = directory.path().join("target");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='identity-probe'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    fs::write(root.join("build.rs"), "mod identity; fn main() { println!(\"cargo::rerun-if-changed=identity.rs\"); identity::export(std::path::Path::new(&std::env::var(\"CARGO_MANIFEST_DIR\").unwrap())); }").unwrap();
    fs::write(
        root.join("identity.rs"),
        include_str!("../../build/identity.rs"),
    )
    .unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { println!(\"{}|{}\", env!(\"RUYIPACK_BUILD_REVISION\"), env!(\"RUYIPACK_BUILD_DIRTY\")); }").unwrap();
    fs::write(root.join("tracked.txt"), "baseline\n").unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "baseline"]);
    let first = git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(build(&root, &target, None), format!("{first}|false"));
    let binary = target.join("debug/identity-probe");
    let built = fs::metadata(&binary).unwrap().modified().unwrap();
    assert_eq!(build(&root, &target, None), format!("{first}|false"));
    assert_eq!(
        fs::metadata(&binary).unwrap().modified().unwrap(),
        built,
        "unchanged inputs must not rebuild"
    );
    fs::write(root.join("tracked.txt"), "modified\n").unwrap();
    assert_eq!(build(&root, &target, None), format!("{first}|true"));
    git(&root, &["add", "tracked.txt"]);
    assert_eq!(build(&root, &target, None), format!("{first}|true"));
    git(&root, &["commit", "-qm", "tracked change"]);
    let second = git(&root, &["rev-parse", "HEAD"]);
    assert_ne!(first, second);
    assert_eq!(build(&root, &target, None), format!("{second}|false"));
    git(&root, &["pack-refs", "--all"]);
    assert_eq!(build(&root, &target, None), format!("{second}|false"));
    git(&root, &["commit", "--allow-empty", "-qm", "metadata only"]);
    let third = git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(build(&root, &target, None), format!("{third}|false"));
    git(&root, &["checkout", "-q", "--detach", &first]);
    assert_eq!(build(&root, &target, None), format!("{first}|false"));
    let worktree = directory.path().join("worktree");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            worktree.to_str().unwrap(),
            &second,
        ],
    );
    assert_eq!(build(&worktree, &target, None), format!("{second}|false"));
    // A source archive inside a repository is not that repository.
    let archive = root.join("archive");
    fs::create_dir_all(archive.join("src")).unwrap();
    for file in ["Cargo.toml", "build.rs", "identity.rs", "src/main.rs"] {
        fs::copy(root.join(file), archive.join(file)).unwrap();
    }
    assert_eq!(build(&archive, &target, None), "|");
    assert_eq!(
        build(&archive, &target, Some((&third, "true"))),
        format!("{third}|true")
    );
    assert_eq!(
        build(&archive, &target, Some((&second, "false"))),
        format!("{second}|false")
    );
    assert_eq!(build(&archive, &target, None), "|");
}

#[test]
fn every_report_command_identifies_its_producer_even_on_input_failure() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        vec!["check", "missing.spec"],
        vec!["inspect", "missing.spec"],
        vec![
            "edit",
            "missing.spec",
            "--set",
            "package.version=2",
            "--check",
        ],
        vec!["gen", "missing", "--offline", "--check"],
        vec!["source-hash", "missing.spec"],
        vec!["verify-sources", "missing.spec"],
    ] {
        let output = command()
            .current_dir(directory.path())
            .args(&args)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        let report = json_line(&output);
        assert_eq!(report["tool"]["name"], "ruyipack", "{report}");
        assert_eq!(report["tool"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(report["tool"].get("revision").is_some());
        assert!(report["tool"].get("dirty").is_some());
    }
}
