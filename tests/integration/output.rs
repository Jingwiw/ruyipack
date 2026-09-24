// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box checks for generated-file destinations and read-only previews.

use super::support::assert_file;

use std::{fs, path::Path, process::Command};

const MANIFEST: &str = include_str!("../../examples/ed/ed.toml");

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
    assert_file(directory.path().join("ed.toml"), MANIFEST);
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
    let help = String::from_utf8_lossy(&conflict.stderr);
    assert!(help.contains("review.spec.new"));
    // gen does accept --output, so its conflict help must keep advertising it.
    assert!(help.contains("--output FILE"), "{help}");
    assert_file(&target, "hand edited\n");

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
    assert_file(target, "hand edited\n");
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

#[cfg(unix)]
#[test]
fn publication_preserves_access_modes_and_uses_umask_for_new_files() {
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    let directory = workspace();
    let target = directory.path().join("ed.spec");
    let control = directory.path().join("creation-control");
    fs::write(&control, "same inherited umask\n").unwrap();
    let created = gen_command(directory.path()).output().unwrap();
    assert!(created.status.success(), "{created:?}");
    assert_eq!(mode(&target), mode(&control));
    let expected = fs::read(&target).unwrap();

    for original_mode in [0o600, 0o640, 0o1640] {
        fs::write(&target, "hand edited\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(original_mode)).unwrap();
        assert_eq!(mode(&target), original_mode);
        let replaced = gen_command(directory.path())
            .arg("--force")
            .output()
            .unwrap();
        assert!(replaced.status.success(), "{replaced:?}");
        assert_eq!(mode(&target), original_mode & 0o777);
        assert_eq!(fs::read(&target).unwrap(), expected);
    }

    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let unchanged = gen_command(directory.path())
        .arg("--force")
        .output()
        .unwrap();
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert_eq!(mode(&target), 0o640);
    assert_eq!(fs::read(&target).unwrap(), expected);

    fs::write(&target, "hand edited\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    let skipped = gen_command(directory.path())
        .arg("--skip-existing")
        .output()
        .unwrap();
    assert!(skipped.status.success(), "{skipped:?}");
    assert_eq!(mode(&target), 0o600);
    assert_eq!(fs::read_to_string(&target).unwrap(), "hand edited\n");
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.toml")).unwrap(),
        MANIFEST
    );
}
