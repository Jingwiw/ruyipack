// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Exposes locked dependency identities to runtime reports.

use std::{env, fs, path::PathBuf};

use toml::Value;

const RPM_SPEC_REPOSITORY: &str = "https://github.com/openRuyi-Project/rpm-spec";
const RPM_SPEC_TOOL_REPOSITORY: &str = "https://github.com/openRuyi-Project/rpm-spec-tool";

fn main() {
    println!("cargo::rerun-if-changed=Cargo.lock");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let lock_path = manifest_dir.join("Cargo.lock");
    let lock_source = fs::read_to_string(&lock_path).expect("read Cargo.lock");
    let lock: Value = toml::from_str(&lock_source).expect("parse Cargo.lock");

    export_git_package(&lock, "rpm-spec", RPM_SPEC_REPOSITORY, "RUYIPACK_RPM_SPEC");
    export_git_package(
        &lock,
        "rpm-spec-analyzer",
        RPM_SPEC_TOOL_REPOSITORY,
        "RUYIPACK_RPM_SPEC_ANALYZER",
    );
}

/// Exports one uniquely resolved Git package from Cargo.lock.
fn export_git_package(lock: &Value, name: &str, repository: &str, prefix: &str) {
    let source_prefix = format!("git+{repository}");
    let packages = lock
        .get("package")
        .and_then(Value::as_array)
        .expect("Cargo.lock contains package entries");
    let mut matches = packages.iter().filter(|package| {
        package.get("name").and_then(Value::as_str) == Some(name)
            && package
                .get("source")
                .and_then(Value::as_str)
                .is_some_and(|source| source.starts_with(&source_prefix))
    });
    let package = matches
        .next()
        .unwrap_or_else(|| panic!("Cargo.lock contains {name} from {repository}"));
    assert!(
        matches.next().is_none(),
        "Cargo.lock contains more than one {name} from {repository}"
    );

    let version = package
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Cargo.lock entry for {name} has a version"));
    let source = package
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Cargo.lock entry for {name} has a source"));
    let revision = source
        .rsplit_once('#')
        .map(|(_, revision)| revision)
        .filter(|revision| {
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| panic!("Cargo.lock entry for {name} has a full Git revision"));

    println!("cargo::rustc-env={prefix}_VERSION={version}");
    println!("cargo::rustc-env={prefix}_REVISION={revision}");
}
