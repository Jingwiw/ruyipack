// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Exposes locked dependency identities to runtime reports.

use std::{env, fs, path::PathBuf};

use toml::Value;

fn main() {
    println!("cargo::rerun-if-changed=Cargo.lock");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let lock_path = manifest_dir.join("Cargo.lock");
    let lock_source = fs::read_to_string(&lock_path).expect("read Cargo.lock");
    let lock: Value = toml::from_str(&lock_source).expect("parse Cargo.lock");

    export_git_package(&lock, "rpm-spec", "RUYIPACK_RPM_SPEC");
    export_git_package(&lock, "rpm-spec-analyzer", "RUYIPACK_RPM_SPEC_ANALYZER");
}

/// Exports one uniquely resolved Git package from Cargo.lock.
fn export_git_package(lock: &Value, name: &str, prefix: &str) {
    let packages = lock
        .get("package")
        .and_then(Value::as_array)
        .expect("Cargo.lock contains package entries");
    let mut matches = packages.iter().filter(|package| {
        package.get("name").and_then(Value::as_str) == Some(name)
            && package
                .get("source")
                .and_then(Value::as_str)
                .is_some_and(|source| source.starts_with("git+"))
    });
    let package = matches
        .next()
        .unwrap_or_else(|| panic!("Cargo.lock contains a Git package named {name}"));
    assert!(
        matches.next().is_none(),
        "Cargo.lock contains more than one Git package named {name}"
    );

    let version = package
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Cargo.lock entry for {name} has a version"));
    let source = package
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Cargo.lock entry for {name} has a source"));
    let (repository, revision) = source
        .strip_prefix("git+")
        .and_then(|source| source.rsplit_once('#'))
        .filter(|(_, revision)| {
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| panic!("Cargo.lock entry for {name} has a full Git revision"));
    let repository = repository
        .split_once('?')
        .map_or(repository, |(url, _)| url);
    assert!(
        !repository.is_empty(),
        "Cargo.lock entry for {name} has a repository"
    );

    println!("cargo::rustc-env={prefix}_REPOSITORY={repository}");

    println!("cargo::rustc-env={prefix}_VERSION={version}");
    println!("cargo::rustc-env={prefix}_REVISION={revision}");
}
