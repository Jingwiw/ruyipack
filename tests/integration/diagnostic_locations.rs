// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Parser diagnostic locations must not contradict their original source.

use super::support::assert_file;

use serde_json::json;
use std::fs;

use super::support;

#[test]
fn inconsistent_conditional_location_does_not_hide_the_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("endpoint.spec");
    let source = "%endif\nx\n";
    fs::write(&path, source).unwrap();
    for (command, exit) in [("check", 1), ("inspect", 0)] {
        let output = support::command()
            .arg(command)
            .arg(&path)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert!(output.stderr.is_empty());
        let report = support::json_line(&output);
        assert_eq!(
            report["parser_diagnostics"],
            json!([
                {"severity":"error", "code":"rpmspec/E0002", "span":null,
                 "message":"`%endif` without matching `%if`", "notes":[]},
                {"severity":"warning", "code":"rpmspec/W0002",
                 "span":{"start_byte":7,"end_byte":8,"start_line":2,"start_column":1,"end_line":2,"end_column":2},
                 "message":"line not recognized", "notes":[]}
            ])
        );
        if command == "check" {
            assert_eq!(report["evidence"]["status"], "incomplete");
            assert_eq!(
                report["evidence"]["incomplete_reasons"],
                json!(["parser-error"])
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
        let output = support::command()
            .arg(command)
            .arg(&path)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        let report = support::json_line(&output);
        assert_eq!(
            report["parser_diagnostics"],
            json!([
                {"severity":"error", "code":"rpmspec/E0002",
                 "span":{"start_byte":0,"end_byte":6,"start_line":1,"start_column":1,"end_line":1,"end_column":7},
                 "message":"`%endif` without matching `%if`", "notes":[]}
            ])
        );
    }
    assert_file(&path, "%endif");
}

#[test]
fn batch_edit_parser_diagnostics_identify_each_candidate_file() {
    let directory = tempfile::tempdir().unwrap();
    let source = format!("%unknown value\n{}", include_str!("../fixtures/ed.spec"));
    let first = directory.path().join("first.spec");
    let second = directory.path().join("second.spec");
    for path in [&first, &second] {
        fs::write(path, &source).unwrap();
    }
    let output = support::command()
        .arg("edit")
        .arg(&first)
        .arg(&second)
        .args(["--field", "package.version", "--check"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = support::output_text(&output.stderr);
    for path in [&first, &second] {
        assert!(
            stderr.contains(&format!(
                "{} (candidate):1:1: warning[rpmspec/W0002]:",
                path.display()
            )),
            "{stderr}"
        );
        assert_file(path, &source);
    }
}
