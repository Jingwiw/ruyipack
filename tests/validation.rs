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

fn run(directory: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ruyipack"))
        .current_dir(directory)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn literal_metadata_checks_reject_invalid_edits_and_keep_sources_unchanged() {
    for (field, original, value, rule) in [
        ("package.name", "Name:           ed", "ed/test", "RPK002"),
        ("package.version", "Version:        1.22.5", "1 2", "RPK002"),
        (
            "spec.release",
            "Release:        %autorelease",
            "1-2",
            "RPK002",
        ),
        (
            "package.url",
            "URL:            https://www.gnu.org/software/ed/",
            "https:/example.org",
            "RPK003",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
        let (tag, _) = original.split_once(':').unwrap();
        let invalid = SPEC.replace(original, &format!("{tag}: {value}"));
        fs::write(directory.path().join("invalid.spec"), &invalid).unwrap();
        let assignment = format!("{field}={value}");
        for args in [
            vec!["check", "invalid.spec"],
            vec!["edit", "ed.spec", "--set", &assignment],
        ] {
            let output = run(directory.path(), &args);
            assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(rule),
                "{output:?}"
            );
            assert!(output.stdout.is_empty());
            assert_eq!(
                fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
                SPEC
            );
            assert_eq!(
                fs::read_to_string(directory.path().join("invalid.spec")).unwrap(),
                invalid
            );
        }
    }
}

#[test]
fn editor_and_saved_drafts_share_metadata_checks() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
    assert!(
        run(
            directory.path(),
            &[
                "edit",
                "ed.spec",
                "--field",
                "package.version",
                "--prepare",
                "drafts"
            ]
        )
        .status
        .success()
    );
    fs::write(
        directory.path().join("drafts/ed.toml"),
        "[package]\nversion = '1 2'\n",
    )
    .unwrap();
    let output = run(directory.path(), &["edit", "--from", "drafts"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("RPK002"));
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC
    );

    #[cfg(unix)]
    {
        fs::write(
            directory.path().join("editor.sh"),
            "#!/bin/sh\nprintf \"[package]\\nversion = '1 2'\\n\" > \"$1\"\n",
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
            .current_dir(directory.path())
            .env("TMPDIR", directory.path())
            .args([
                "edit",
                "ed.spec",
                "--field",
                "package.version",
                "--editor",
                "sh editor.sh",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("RPK002") && error.contains("Drafts retained"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC
        );
    }
}

#[test]
fn metadata_checks_preserve_literal_versions_http_and_unevaluated_macros() {
    let directory = tempfile::tempdir().unwrap();
    for (field, value) in [
        ("package.name", "_ed"),
        ("package.version", "2.0~rc1^20260917"),
        ("package.version", "%{upstream_version}"),
        ("spec.release", "1%{?dist}"),
        ("package.url", "http://example.org/project"),
    ] {
        fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
        let output = run(
            directory.path(),
            &[
                "edit",
                "ed.spec",
                "--set",
                &format!("{field}={value}"),
                "--stdout",
            ],
        );
        assert!(output.status.success(), "{field}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(value));
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC
        );
    }
    fs::write(
        directory.path().join("ed.spec"),
        SPEC.replace("Name:           ed", "Epoch: 0\nName:           ed"),
    )
    .unwrap();
    assert!(
        run(directory.path(), &["check", "ed.spec"])
            .status
            .success()
    );
}
