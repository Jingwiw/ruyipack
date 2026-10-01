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
    // HEAD selects the branch; unrelated local/remote refs do not identify this build.
    for name in ["HEAD", "index", "packed-refs"] {
        let path = git(root, &["rev-parse", "--git-path", name])?;
        watch(&root.join(path.trim()), name);
    }
    if let Some(reference) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        let path = git(root, &["rev-parse", "--git-path", reference.trim()])?;
        watch(&root.join(path.trim()), "current-ref");
    }
    let revision = git(root, &["rev-parse", "--verify", "HEAD"])?;
    let revision = revision.trim().to_owned();
    if !full_revision(&revision) {
        return None;
    }
    let files = git(root, &["ls-files", "-z"])?;
    for (index, file) in files.split_terminator('\0').enumerate() {
        if file.contains(['\n', '\r']) {
            return Some((Some(revision), None));
        }
        watch(&root.join(file), &format!("file-{index}"));
    }
    // Like git describe --dirty, this records tracked changes, not unrelated untracked files.
    let dirty = git(root, &["status", "--porcelain", "--untracked-files=no"])
        .map(|status| (!status.is_empty()).to_string());
    Some((Some(revision), dirty))
}

fn watch(path: &Path, key: &str) {
    #[cfg(unix)]
    if !path.exists() {
        // Cargo treats a missing file as perpetually dirty. A directory containing
        // a dangling link remains stable, but Cargo follows it when the input returns.
        // Never watch the real parent: it may contain target/ or unrelated Git refs.
        let directory = std::path::PathBuf::from(env::var_os("OUT_DIR").expect("build output"))
            .join("identity-watch")
            .join(key);
        fs::create_dir_all(&directory).expect("create identity watch directory");
        let link = directory.join("input");
        if fs::read_link(&link).ok().as_deref() != Some(path) {
            if link.symlink_metadata().is_ok() {
                fs::remove_file(&link).expect("replace identity watch link");
            }
            std::os::unix::fs::symlink(path, &link).expect("link identity input");
        }
        println!("cargo::rerun-if-changed={}", directory.display());
        return;
    }
    #[cfg(not(unix))]
    let _ = key;
    println!("cargo::rerun-if-changed={}", path.display());
}
