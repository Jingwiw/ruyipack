// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Build-time identity only; runtime reports do not invoke Git.

use std::{env, fs, path::Path, process::Command};

pub fn export(root: &Path) {
    println!("cargo::rerun-if-env-changed=RUYIPACK_SOURCE_REVISION");
    println!("cargo::rerun-if-env-changed=RUYIPACK_SOURCE_DIRTY");
    let (revision, dirty) = if let Ok(revision) = env::var("RUYIPACK_SOURCE_REVISION") {
        assert!(
            full_revision(&revision),
            "RUYIPACK_SOURCE_REVISION must be a full Git object ID"
        );
        let dirty = env::var("RUYIPACK_SOURCE_DIRTY").unwrap_or_default();
        assert!(
            matches!(dirty.as_str(), "" | "true" | "false"),
            "RUYIPACK_SOURCE_DIRTY must be true, false, or unset"
        );
        (Some(revision), (!dirty.is_empty()).then_some(dirty))
    } else {
        assert!(
            env::var_os("RUYIPACK_SOURCE_DIRTY").is_none(),
            "RUYIPACK_SOURCE_DIRTY requires RUYIPACK_SOURCE_REVISION"
        );
        local(root).unwrap_or_default()
    };
    println!(
        "cargo::rustc-env=RUYIPACK_BUILD_REVISION={}",
        revision.unwrap_or_default()
    );
    println!(
        "cargo::rustc-env=RUYIPACK_BUILD_DIRTY={}",
        dirty.unwrap_or_default()
    );
}

fn full_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

fn local(root: &Path) -> Option<(Option<String>, Option<String>)> {
    // An archive nested inside another checkout must never inherit that checkout's identity.
    if !root.join(".git").exists() {
        return None;
    }
    let top = git(root, &["rev-parse", "--show-toplevel"])?;
    if fs::canonicalize(top.trim()).ok()? != fs::canonicalize(root).ok()? {
        return None;
    }
    if root.join(".git").is_file() {
        println!("cargo::rerun-if-changed=.git");
    }
    // Git resolves worktree metadata, loose refs and the common packed-ref store.
    // Do not watch all of .git (objects/logs) or the source root (target/).
    for name in ["HEAD", "index", "refs", "packed-refs"] {
        let path = git(root, &["rev-parse", "--git-path", name])?;
        let path = root.join(path.trim());
        if path.exists() {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
    let revision = git(root, &["rev-parse", "--verify", "HEAD"])?;
    let revision = revision.trim().to_owned();
    if !full_revision(&revision) {
        return None;
    }
    let files = git(root, &["ls-files", "-z"])?;
    for file in files.split_terminator('\0') {
        if file.contains(['\n', '\r']) {
            return Some((Some(revision), None));
        }
        println!("cargo::rerun-if-changed={}", root.join(file).display());
    }
    // Like git describe --dirty, this records tracked changes, not unrelated untracked files.
    let dirty = git(root, &["status", "--porcelain", "--untracked-files=no"])
        .map(|status| (!status.is_empty()).to_string());
    Some((Some(revision), dirty))
}
