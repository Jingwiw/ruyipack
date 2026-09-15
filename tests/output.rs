// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box checks for generated-file destinations and read-only previews.

use std::{fs, path::Path, process::Command};

const MANIFEST: &str = include_str!("../examples/ed/ed.toml");

fn workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("create workspace");
    fs::write(directory.path().join("ed.toml"), MANIFEST).expect("write manifest");
    directory
}

fn gen_command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruyipack"));
    command.current_dir(directory).args(["gen", "ed"]);
    command
}

#[test]
fn diff_never_writes_new_changed_or_identical_targets() {
    let directory = workspace();
    let path = directory.path().join("ed.spec");
    let preview = gen_command(directory.path())
        .arg("--stdout")
        .output()
        .unwrap();
    assert!(preview.status.success());
    assert!(!path.exists());

    let new = gen_command(directory.path())
        .arg("--diff")
        .output()
        .unwrap();
    assert!(new.status.success());
    assert!(new.stdout.starts_with(b"--- /dev/null\n"));
    assert!(new.stderr.is_empty());
    assert!(!path.exists());

    let spaced = gen_command(directory.path())
        .args(["--diff", "-o", "ed review.spec"])
        .output()
        .unwrap();
    assert!(spaced.status.success());
    assert!(
        spaced
            .stdout
            .starts_with(b"--- /dev/null\n+++ ed review.spec\t\n")
    );
    assert!(!directory.path().join("ed review.spec").exists());

    let edited = b"# local edits\n";
    fs::write(&path, edited).unwrap();
    let changed = gen_command(directory.path())
        .arg("--diff")
        .output()
        .unwrap();
    assert!(changed.status.success());
    assert!(!changed.stdout.is_empty());
    assert_eq!(fs::read(&path).unwrap(), edited);

    fs::write(&path, &preview.stdout).unwrap();
    let identical = gen_command(directory.path())
        .arg("--diff")
        .output()
        .unwrap();
    assert!(identical.status.success());
    assert!(identical.stdout.is_empty());
    assert_eq!(fs::read(&path).unwrap(), preview.stdout);
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.toml")).unwrap(),
        MANIFEST
    );
}

#[test]
fn output_selects_a_cwd_relative_file_and_uses_the_same_overwrite_policy() {
    let directory = workspace();
    fs::create_dir(directory.path().join("nested")).unwrap();
    fs::rename(
        directory.path().join("ed.toml"),
        directory.path().join("nested/ed.toml"),
    )
    .unwrap();
    let args = [
        "--manifest",
        "nested/ed.toml",
        "--output",
        "review.spec.new",
    ];
    let generated = gen_command(directory.path()).args(args).output().unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let target = directory.path().join("review.spec.new");
    let expected = fs::read(&target).unwrap();
    assert!(!directory.path().join("nested/ed.spec").exists());
    assert!(!directory.path().join("ed.spec").exists());

    fs::write(&target, "hand edited\n").unwrap();
    let conflict = gen_command(directory.path()).args(args).output().unwrap();
    assert_eq!(conflict.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("review.spec.new"));
    assert_eq!(fs::read_to_string(&target).unwrap(), "hand edited\n");

    let replaced = gen_command(directory.path())
        .args(args)
        .arg("-f")
        .output()
        .unwrap();
    assert!(replaced.status.success());
    assert_eq!(fs::read(&target).unwrap(), expected);
    let diff = gen_command(directory.path())
        .args(args)
        .arg("--diff")
        .output()
        .unwrap();
    assert!(diff.status.success());
    assert!(diff.stdout.is_empty());
}

#[test]
fn skip_existing_keeps_edits_and_creates_missing_targets() {
    let directory = workspace();
    let target = directory.path().join("ed.spec");
    let created = gen_command(directory.path())
        .arg("--skip-existing")
        .output()
        .unwrap();
    assert!(created.status.success());
    assert!(target.is_file());
    fs::write(&target, "hand edited\n").unwrap();
    let kept = gen_command(directory.path())
        .arg("--skip-existing")
        .output()
        .unwrap();
    assert!(kept.status.success());
    assert!(kept.stdout.is_empty());
    assert_eq!(fs::read_to_string(target).unwrap(), "hand edited\n");
}

#[test]
fn clap_rejects_incompatible_output_options_before_reading_a_manifest() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        ["--force", "--skip-existing"].as_slice(),
        &["--diff", "--force"],
        &["--diff", "--skip-existing"],
        &["--stdout", "--diff"],
        &["--stdout", "--force"],
        &["--stdout", "--skip-existing"],
        &["--stdout", "--output=other.spec"],
    ] {
        for ordered in [args.to_vec(), args.iter().rev().copied().collect()] {
            let result = gen_command(directory.path())
                .args(ordered)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(2), "{args:?}");
            assert!(result.stdout.is_empty());
            assert!(String::from_utf8_lossy(&result.stderr).contains("cannot be used with"));
        }
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn output_protection_applies_to_the_actual_input_and_parent_directory() {
    let directory = workspace();
    let aliased = gen_command(directory.path())
        .args(["-o", "ed.toml", "--force"])
        .output()
        .unwrap();
    assert_eq!(aliased.status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.toml")).unwrap(),
        MANIFEST
    );
    let missing_parent = gen_command(directory.path())
        .args(["-o", "missing/ed.spec"])
        .output()
        .unwrap();
    assert_eq!(missing_parent.status.code(), Some(1));
    assert!(!directory.path().join("missing").exists());
    assert!(!directory.path().join("ed.spec").exists());
}
