// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Parser diagnostic locations must not contradict their original source.

use super::support::assert_file;

use std::fs;

use super::support;

#[test]
fn inconsistent_conditional_location_does_not_hide_the_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("endpoint.spec");
    let source = "%endif\nx\n";
    fs::write(&path, source).unwrap();
    for (command, exit) in [("check", 1), ("inspect", 0)] {
        let output = support::spec_report(command, &path);
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert!(output.stderr.is_empty());
        let report = support::machine_report(&output);
        assert_eq!(
            report["parser_diagnostics"],
            toml::Value::Array(vec![
                toml::Value::Table(toml::toml! {
                    "severity" = "error"
                    "code" = "rpmspec/E0002"
                    "message" = "`%endif` without matching `%if`"
                    "notes" = []
                }),
                toml::Value::Table(toml::toml! {
                    "severity" = "warning"
                    "code" = "rpmspec/W0002"
                    "span" = { "start_byte" = 7, "end_byte" = 8, "start_line" = 2, "start_column" = 1, "end_line" = 2, "end_column" = 2 }
                    "message" = "line not recognized"
                    "notes" = []
                })
            ])
        );
        if command == "check" {
            assert_eq!(report["evidence"]["status"].as_str(), Some("incomplete"));
            assert_eq!(
                report["evidence"]["incomplete_reasons"],
                toml::Value::Array(vec![toml::Value::from("parser-error")])
            );
        }
    }
    assert_file(&path, source);
}

#[test]
fn consistent_conditional_location_is_not_removed_by_error_code() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("endpoint.spec");
    fs::write(&path, "%endif").unwrap();
    for (command, exit) in [("check", 1), ("inspect", 0)] {
        let output = support::spec_report(command, &path);
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        let report = support::machine_report(&output);
        assert_eq!(
            report["parser_diagnostics"],
            toml::Value::Array(vec![toml::Value::Table(toml::toml! {
                "severity" = "error"
                "code" = "rpmspec/E0002"
                "span" = { "start_byte" = 0, "end_byte" = 6, "start_line" = 1, "start_column" = 1, "end_line" = 1, "end_column" = 7 }
                "message" = "`%endif` without matching `%if`"
                "notes" = []
            })])
        );
    }
    assert_file(&path, "%endif");
}

#[test]
fn batch_edit_parser_diagnostics_distinguish_same_named_candidate_files() {
    let directory = tempfile::tempdir().unwrap();
    let source = format!("%unknown value\n{}", include_str!("../fixtures/ed.spec"));
    for name in ["first", "second"] {
        fs::create_dir(directory.path().join(name)).unwrap();
    }
    let first = directory.path().join("first/pkg.spec");
    let second = directory.path().join("second/pkg.spec");
    for path in [&first, &second] {
        fs::write(path, &source).unwrap();
    }
    let output = support::command()
        .current_dir(directory.path())
        .arg("edit")
        .arg("--spec")
        .arg(&first)
        .arg("--spec")
        .arg(&second)
        .args(["--set", "package.version=1.22.5", "--check"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = support::output_text(&output.stderr);
    let sections: Vec<_> = stderr.split("candidate static blockers:").skip(1).collect();
    assert_eq!(sections.len(), 2, "{stderr}");
    for (path, section) in [&first, &second].into_iter().zip(sections) {
        assert!(
            stderr.contains(&format!(
                "[INFO] {}: candidate static blockers:",
                path.strip_prefix(directory.path()).unwrap().display()
            )),
            "{stderr}"
        );
        assert!(
            section.contains("[WARN] spec[1:1] [rpmspec/W0002]:"),
            "{stderr}"
        );
        assert_file(path, &source);
    }
}

#[test]
fn parser_input_boundaries_are_reported_without_panicking_or_writing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("boundary.spec");
    for (source, limited) in [
        ("BuildRequires: é 中 😀\n".to_owned(), false),
        (
            format!(
                "Version: {}1{}\n",
                "%{expand:".repeat(2048),
                "}".repeat(2048)
            ),
            true,
        ),
        (
            format!(
                "{}Version: 1\n{}",
                "%if 1\n".repeat(1024),
                "%endif\n".repeat(1024)
            ),
            true,
        ),
    ] {
        fs::write(&path, &source).unwrap();
        for (command, exit) in [("check", 1), ("inspect", 0)] {
            let output = support::command()
                .arg(command)
                .arg("--spec")
                .arg(&path)
                .args([
                    "--format",
                    if limited && command == "inspect" {
                        "human"
                    } else {
                        "toml"
                    },
                ])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(exit), "{output:?}");
            if limited && command == "inspect" {
                assert!(support::output_text(&output.stderr).contains("rpmspec/E0011"));
                continue;
            }
            assert!(output.stderr.is_empty(), "{output:?}");
            let report = support::machine_report(&output);
            assert_eq!(
                report["parser_diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["code"].as_str() == Some("rpmspec/E0011")),
                limited
            );
            if limited && command == "check" {
                assert_eq!(report["evidence"]["status"].as_str(), Some("incomplete"));
            }
        }
        if limited {
            let output = support::command()
                .arg("edit")
                .arg("--spec")
                .arg(&path)
                .args(["--set", "package.version=2"])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{output:?}");
        }
        assert_file(&path, &source);
    }
}
