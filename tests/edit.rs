// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only field selection and its matching JSON Schema.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const SOURCE: &str = include_str!("fixtures/ed.spec");

fn fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), SOURCE).unwrap();
    directory
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruyipack"));
    command.arg("edit").current_dir(directory);
    command
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn unchanged(directory: &Path) {
    assert_eq!(
        fs::read_to_string(directory.join("ed.spec")).unwrap(),
        SOURCE
    );
}

#[test]
fn selected_view_and_schema_have_the_same_narrow_shape() {
    let directory = fixture();
    let output = command(directory.path())
        .args(["ed.spec", "--view", "--field", "package.version"])
        .output()
        .unwrap();
    success(&output);
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(document.len(), 1);
    assert_eq!(document["package"].as_table().unwrap().len(), 1);
    let output = command(directory.path())
        .args(["ed.spec", "--schema", "--field", "package.version"])
        .output()
        .unwrap();
    success(&output);
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    let package = &schema["properties"]["package"];
    assert_eq!(package["additionalProperties"], false);
    assert_eq!(package["properties"].as_object().unwrap().len(), 1);
    assert_eq!(package["properties"]["version"]["type"], "string");
    assert!(
        package["properties"]["version"]["description"]
            .as_str()
            .unwrap()
            .contains("Version")
    );
    unchanged(directory.path());
}

#[test]
fn group_selection_keeps_descendants_without_duplicating_overlaps() {
    let directory = fixture();
    let output = command(directory.path())
        .args([
            "ed.spec",
            "--view",
            "--field",
            "package.files",
            "--field",
            "package.files.doc",
        ])
        .output()
        .unwrap();
    success(&output);
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(document["package"].as_table().unwrap().len(), 1);
    let files = document["package"]["files"].as_table().unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        files.contains_key("doc") && files.contains_key("license") && files.contains_key("entries")
    );
    let bad = command(directory.path())
        .args(["ed.spec", "--view", "--field", "package.unknown"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(bad.stdout.is_empty());
}
