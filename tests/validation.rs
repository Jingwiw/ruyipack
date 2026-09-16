// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Entry-point checks for shared package validation.

use std::{fs, process::Command};

const SPEC: &str = include_str!("fixtures/ed.spec");
const MANIFEST: &str = include_str!("../examples/ed/ed.toml");
const LICENSE: &str = "GPL-3.0-or-later AND LGPL-2.1-or-later";

#[test]
fn invalid_license_is_rejected_by_check_gen_and_edit_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let invalid = "Definitely-Not-A-License";
    fs::write(dir.path().join("ed.spec"), SPEC).unwrap();
    fs::write(
        dir.path().join("invalid.spec"),
        SPEC.replace(LICENSE, invalid),
    )
    .unwrap();
    fs::write(
        dir.path().join("ed.toml"),
        MANIFEST.replace(LICENSE, invalid),
    )
    .unwrap();
    for args in [
        vec!["check", "invalid.spec"],
        vec!["gen", "ed", "--force"],
        vec![
            "edit",
            "ed.spec",
            "--set",
            "package.license=Definitely-Not-A-License",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
            .current_dir(dir.path())
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("RPK001") && error.contains("package.license"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("ed.spec")).unwrap(),
            SPEC
        );
    }
}

#[test]
fn license_checks_visit_subpackages_and_conditions_without_expanding_macros() {
    let dir = tempfile::tempdir().unwrap();
    for (extra, status, reason, severities) in [
        (
            "%if 0\nLicense: Definitely-Not-A-License\n%endif\n",
            "fail",
            None,
            &["deny"][..],
        ),
        (
            "%package tools\nSummary: Tools\nLicense: Definitely-Not-A-License\n",
            "fail",
            None,
            &["deny"][..],
        ),
        (
            "License: %{package_license}\n",
            "incomplete",
            Some("unresolved-license"),
            &["warn"][..],
        ),
        (
            "%if 0\nLicense: Definitely-Not-A-License\n%else\nLicense: %{package_license}\n%endif\n",
            "fail",
            None,
            &["deny", "warn"][..],
        ),
    ] {
        let (head, sections) = SPEC.split_once("%description").unwrap();
        let source = if extra == "License: %{package_license}\n" {
            SPEC.replace(LICENSE, "%{package_license}")
        } else {
            format!("{head}{extra}%description{sections}")
        };
        fs::write(dir.path().join("ed.spec"), &source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
            .current_dir(dir.path())
            .args(["check", "ed.spec", "--format", "json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{extra}: {output:?}");
        assert!(output.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["evidence"]["status"], status, "{report}");
        assert_eq!(report["evidence"]["reason"].as_str(), reason, "{report}");
        let findings: Vec<_> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["code"] == "RPK001")
            .collect();
        assert_eq!(findings.len(), severities.len(), "{report}");
        for (finding, severity) in findings.iter().zip(severities) {
            assert_eq!(finding["producer"], "ruyipack");
            assert_eq!(finding["severity"], *severity);
            let span = &finding["span"];
            let start = usize::try_from(span["start_byte"].as_u64().unwrap()).unwrap();
            let end = usize::try_from(span["end_byte"].as_u64().unwrap()).unwrap();
            assert!(source[start..end].starts_with("License:"));
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("ed.spec")).unwrap(),
            source
        );
    }
}
