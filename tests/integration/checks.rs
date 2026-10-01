// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Entry-point checks for shared package validation.

use super::support::{assert_file, authoring_workspace, run, success};

use std::{fs, process::Command};

const SPEC: &str = include_str!("../fixtures/ed.spec");
const MANIFEST: &str = include_str!("../../examples/ed/ed.toml");
const LICENSE: &str = "GPL-3.0-or-later AND LGPL-2.1-or-later";
const SPEC_LICENSE: &str = concat!("# SPDX-License-", "Identifier: MulanPSL-2.0\n");

#[test]
fn spec_license_uses_spdx_checks_without_changing_the_package_license() {
    let dir = tempfile::tempdir().unwrap();
    let invalid = SPEC.replace("MulanPSL-2.0", "not-a-real-license");
    fs::write(dir.path().join("invalid.spec"), &invalid).unwrap();
    fs::write(dir.path().join("ed.spec"), SPEC).unwrap();
    let output = run(
        dir.path(),
        &["check", "--spec=invalid.spec", "--format", "toml"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report = super::support::machine_report(&output);
    let findings = report["findings"].as_array().unwrap();
    let finding = findings
        .iter()
        .find(|f| f["code"].as_str() == Some("RPK001"))
        .unwrap();
    assert!(
        finding["message"]
            .as_str()
            .unwrap()
            .contains("spec.license")
    );
    let span = &finding["span"];
    let start = span["start_byte"].as_integer().unwrap() as usize;
    let end = span["end_byte"].as_integer().unwrap() as usize;
    assert_eq!(
        &invalid[start..end],
        SPEC_LICENSE.replace("MulanPSL-2.0", "not-a-real-license")
    );
    let output = run(
        dir.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--apply",
            "--set",
            "spec.license=not-a-real-license",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("RPK001") && error.contains("spec.license"),
        "{error}"
    );
    assert_file(dir.path().join("ed.spec"), SPEC);
    assert_file(dir.path().join("invalid.spec"), &invalid);

    let expression = "MIT AND (Apache-2.0 OR GPL-2.0-only WITH Classpath-exception-2.0)";
    let assignment = format!("spec.license={expression}");
    let output = run(
        dir.path(),
        &["edit", "--spec=ed.spec", "--set", &assignment, "--apply"],
    );
    assert!(output.status.success(), "{output:?}");
    assert_file(
        dir.path().join("ed.spec"),
        &(SPEC.replace("MulanPSL-2.0", expression)),
    );
    assert!(
        run(dir.path(), &["check", "--spec=ed.spec"])
            .status
            .success()
    );
}

#[test]
fn spec_license_header_scope_keeps_missing_and_duplicate_edit_policy() {
    let dir = tempfile::tempdir().unwrap();
    let second_header = concat!("# SPDX-License-", "Identifier: MIT\n");
    for source in [
        SPEC.replace(SPEC_LICENSE, ""),
        SPEC.replace(SPEC_LICENSE, &format!("{SPEC_LICENSE}{second_header}")),
    ] {
        let _ = fs::remove_dir_all(dir.path().join(".ruyipack-stage"));
        fs::write(dir.path().join("ed.spec"), &source).unwrap();
        let checked = run(dir.path(), &["check", "--spec=ed.spec"]);
        assert!(checked.status.success(), "{checked:?}");
        let edited = run(
            dir.path(),
            &["edit", "--spec=ed.spec", "--set", "spec.license=MIT"],
        );
        assert_eq!(edited.status.code(), Some(1), "{edited:?}");
        assert_file(dir.path().join("ed.spec"), &source);
    }
    let source = SPEC.replace(
        SPEC_LICENSE,
        &format!(
            "{SPEC_LICENSE}{}",
            second_header.replace("MIT", "not-a-real-license")
        ),
    );
    fs::write(dir.path().join("ed.spec"), source).unwrap();
    let output = run(dir.path(), &["check", "--spec=ed.spec"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("spec.license"));

    let script = format!(
        "%build\ncat <<'EOF'\n{}EOF\n\n%files\n",
        second_header.replace("MIT", "not-a-real-license")
    );
    fs::write(
        dir.path().join("ed.spec"),
        SPEC.replace("%files\n", &script),
    )
    .unwrap();
    let output = run(dir.path(), &["check", "--spec=ed.spec"]);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn invalid_license_is_rejected_by_check_gen_and_edit_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    success(&run(dir.path(), &["init"]));
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
    authoring_workspace(
        dir.path(),
        "review",
        "ed",
        &fs::read_to_string(dir.path().join("ed.toml")).unwrap(),
    );
    for args in [
        vec!["check", "--spec=invalid.spec"],
        vec!["gen", "review", "--offline", "--spec=auto", "--force"],
        vec![
            "edit",
            "--spec=ed.spec",
            "--apply",
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
        assert_file(dir.path().join("ed.spec"), SPEC);
    }
}

#[test]
fn license_checks_visit_subpackages_and_conditions_without_expanding_macros() {
    let dir = tempfile::tempdir().unwrap();
    for (extra, status, reasons, severities) in [
        (
            "%if 0\nLicense: Definitely-Not-A-License\n%endif\n",
            "fail",
            &[][..],
            &["deny"][..],
        ),
        (
            "%package tools\nSummary: Tools\nLicense: Definitely-Not-A-License\n",
            "fail",
            &[][..],
            &["deny"][..],
        ),
        (
            "License: %{package_license}\n",
            "incomplete",
            &["unresolved-license"][..],
            &["warn"][..],
        ),
        (
            "%if 0\nLicense: Definitely-Not-A-License\n%else\nLicense: %{package_license}\n%endif\n",
            "fail",
            &["unresolved-license"][..],
            &["deny", "warn"][..],
        ),
    ] {
        let (head, sections) = SPEC.split_once("%description").unwrap();
        let source = if extra == "License: %{package_license}\n" {
            SPEC.replace(LICENSE, "%{package_license}")
        } else {
            format!("{head}{extra}%description{sections}")
        };
        let _ = fs::remove_dir_all(dir.path().join(".ruyipack-stage"));
        fs::write(dir.path().join("ed.spec"), &source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ruyipack"))
            .current_dir(dir.path())
            .args(["check", "--spec=ed.spec", "--format", "toml"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{extra}: {output:?}");
        assert!(output.stderr.is_empty());
        let report = super::support::machine_report(&output);
        assert_eq!(
            report["evidence"]["status"].as_str(),
            Some(status),
            "{report}"
        );
        assert_eq!(
            report["evidence"]["incomplete_reasons"],
            toml::Value::try_from(reasons).unwrap(),
            "{report}"
        );
        let findings: Vec<_> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["code"].as_str() == Some("RPK001"))
            .collect();
        assert_eq!(findings.len(), severities.len(), "{report}");
        for (finding, severity) in findings.iter().zip(severities) {
            assert_eq!(finding["producer"].as_str(), Some("ruyipack"));
            assert_eq!(finding["severity"].as_str(), Some(*severity));
            let span = &finding["span"];
            let start = usize::try_from(span["start_byte"].as_integer().unwrap()).unwrap();
            let end = usize::try_from(span["end_byte"].as_integer().unwrap()).unwrap();
            assert!(source[start..end].starts_with("License:"));
        }
        assert_file(dir.path().join("ed.spec"), &source);
    }
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
            vec!["check", "--spec=invalid.spec"],
            vec!["edit", "--spec=ed.spec", "--set", &assignment, "--apply"],
        ] {
            let output = run(directory.path(), &args);
            assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(rule),
                "{output:?}"
            );
            assert!(output.stdout.is_empty());
            assert_file(directory.path().join("ed.spec"), SPEC);
            assert_file(directory.path().join("invalid.spec"), &invalid);
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
                "--spec=ed.spec",
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
    let output = run(directory.path(), &["edit", "--from", "drafts", "--apply"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("RPK002"));
    assert_file(directory.path().join("ed.spec"), SPEC);

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
                "--spec=ed.spec",
                "--field",
                "package.version",
                "--editor",
                "sh editor.sh",
                "--apply",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("RPK002") && error.contains("stage"),
            "{error}"
        );
        assert_file(directory.path().join("ed.spec"), SPEC);
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
                "--spec=ed.spec",
                "--set",
                &format!("{field}={value}"),
                "--stdout",
            ],
        );
        assert!(output.status.success(), "{field}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(value));
        assert_file(directory.path().join("ed.spec"), SPEC);
    }
    fs::write(
        directory.path().join("ed.spec"),
        SPEC.replace("Name:           ed", "Epoch: 0\nName:           ed"),
    )
    .unwrap();
    assert!(
        run(directory.path(), &["check", "--spec=ed.spec"])
            .status
            .success()
    );
}

#[test]
fn autotools_requirements_are_checked_by_check_gen_and_saved_edits() {
    let directory = tempfile::tempdir().unwrap();
    success(&run(directory.path(), &["init"]));
    fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
    fs::write(
        directory.path().join("invalid.spec"),
        SPEC.replace("BuildRequires:  autoconf\n", ""),
    )
    .unwrap();
    fs::write(
        directory.path().join("ed.toml"),
        MANIFEST.replace("\"autoconf\", ", ""),
    )
    .unwrap();
    assert!(
        run(
            directory.path(),
            &[
                "edit",
                "--spec=ed.spec",
                "--field",
                "build-requires",
                "--prepare",
                "drafts"
            ]
        )
        .status
        .success()
    );
    let draft = directory.path().join("drafts/ed.toml");
    let mut doc: toml::Table = toml::from_str(&fs::read_to_string(&draft).unwrap()).unwrap();
    doc["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .retain(|value| value.as_str() != Some("autoconf"));
    fs::write(&draft, toml::to_string_pretty(&doc).unwrap()).unwrap();
    let checked = run(
        directory.path(),
        &["edit", "--from", "drafts", "--check", "--format", "toml"],
    );
    assert_eq!(checked.status.code(), Some(1));
    assert!(checked.stderr.is_empty());
    let report = super::support::machine_report(&checked);
    assert_eq!(
        report["files"][0]["introduced_static_blockers"].as_bool(),
        Some(true)
    );
    assert_eq!(
        report["files"][0]["baseline_report"]["evidence"]["status"].as_str(),
        Some("pass")
    );
    authoring_workspace(
        directory.path(),
        "review",
        "ed",
        &fs::read_to_string(directory.path().join("ed.toml")).unwrap(),
    );
    for args in [
        vec!["check", "--spec=invalid.spec"],
        vec!["gen", "review", "--offline", "--spec=auto", "--force"],
        vec!["edit", "--from", "drafts", "--apply"],
    ] {
        let output = run(directory.path(), &args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("RPK004") && error.contains("autoconf"),
            "{error}"
        );
        assert_file(directory.path().join("ed.spec"), SPEC);
    }
}

#[test]
fn build_requirements_do_not_guess_conditions_macros_or_unknown_systems() {
    let directory = tempfile::tempdir().unwrap();
    for dependency in [
        "%if 0\nBuildRequires: autoconf\n%endif",
        "BuildRequires: %{autoconf_requirement}",
        "BuildRequires: (autoconf or other-tool)",
    ] {
        fs::write(
            directory.path().join("ed.spec"),
            SPEC.replace("BuildRequires:  autoconf", dependency),
        )
        .unwrap();
        let output = run(
            directory.path(),
            &["check", "--spec=ed.spec", "--format", "toml"],
        );
        let report = super::support::machine_report(&output);
        assert_eq!(
            report["evidence"]["status"].as_str(),
            Some("incomplete"),
            "{report}"
        );
        assert_eq!(
            report["evidence"]["incomplete_reasons"],
            toml::Value::Array(vec![toml::Value::from("unresolved-build-requirements")])
        );
        assert_eq!(report["findings"][0]["severity"].as_str(), Some("warn"));
        assert_eq!(output.status.code(), Some(1));
    }
    for system in ["meson", "custom-system", "%{build_system}"] {
        let source = SPEC
            .replace(
                "BuildSystem:    autotools",
                &format!("BuildSystem: {system}"),
            )
            .replace("BuildRequires:  autoconf\n", "");
        fs::write(directory.path().join("ed.spec"), source).unwrap();
        let output = run(
            directory.path(),
            &["check", "--spec=ed.spec", "--format", "toml"],
        );
        assert!(output.status.success());
        let report = super::support::machine_report(&output);
        assert_eq!(
            report["evidence"]["incomplete_reasons"],
            toml::Value::Array(vec![])
        );
    }
}

#[test]
fn independent_incomplete_reasons_survive_each_other_and_confirmed_failures() {
    let directory = tempfile::tempdir().unwrap();
    let incomplete = SPEC
        .replace(LICENSE, "%{package_license}")
        .replace(
            "BuildRequires:  autoconf",
            "%if 0\nBuildRequires: autoconf\n%endif",
        )
        .replace(
            "%description",
            "%if 0\nLicense: %{secondary_license}\n%endif\n%description",
        );
    for (source, status) in [
        (incomplete.clone(), "incomplete"),
        (
            incomplete.replace(
                "%description",
                "License: Definitely-Not-A-License\n%description",
            ),
            "fail",
        ),
    ] {
        let path = directory.path().join("ed.spec");
        fs::write(&path, &source).unwrap();
        let output = run(
            directory.path(),
            &["check", "--spec=ed.spec", "--format", "toml"],
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        let report = super::support::machine_report(&output);
        assert_eq!(report["format_version"].as_integer(), Some(2));
        assert_eq!(report["evidence"]["status"].as_str(), Some(status));
        assert_eq!(
            report["evidence"]["incomplete_reasons"],
            toml::Value::Array(vec![
                toml::Value::from("unresolved-license"),
                toml::Value::from("unresolved-build-requirements")
            ])
        );
        let findings = report["findings"].as_array().unwrap();
        for rule in ["RPK001", "RPK004"] {
            assert!(findings.iter().any(
                |f| f["code"].as_str() == Some(rule) && f["severity"].as_str() == Some("warn")
            ));
        }
        assert_eq!(
            findings
                .iter()
                .any(|f| f["severity"].as_str() == Some("deny")),
            status == "fail"
        );
        let human = run(directory.path(), &["check", "--spec=ed.spec"]);
        assert_eq!(human.status.code(), Some(1));
        let diagnostics = String::from_utf8(human.stderr).unwrap();
        assert!(diagnostics.contains("license expressions require RPM evaluation"));
        assert!(diagnostics.contains("build requirements require RPM evaluation"));
        assert_file(&path, &source);
    }
}

#[test]
fn static_policy_changes_admission_without_inventing_source_facts() {
    let dir = tempfile::tempdir().unwrap();
    let start = SPEC.find("#!RemoteAsset:").unwrap();
    let end = SPEC.find("BuildSystem:").unwrap();
    let hash = "a".repeat(64);
    for (declarations, violation, unknown) in [
        (
            format!("#!RemoteAsset:  sha256:{hash}\nSource0: https://example.org/archive\n"),
            false,
            false,
        ),
        ("Source0: https://example.org/archive\n".into(), true, false),
        (
            "%global archive https://example.org/archive\nSource0: %{archive}\n".into(),
            true,
            false,
        ),
        (
            "#!RemoteAsset:  sha256:INVALID\nSource0: https://example.org/archive\n".into(),
            true,
            false,
        ),
        (
            "#!RemoteAsset: md5:abc\nSource0: https://example.org/archive\n".into(),
            true,
            false,
        ),
        ("Source0: local.tar.gz\n".into(), false, false),
        (
            "%if 0\nSource0: https://example.org/archive\n%endif\n".into(),
            false,
            false,
        ),
        ("Source0: %{unknown}\n".into(), false, true),
        ("%include absent.inc\n".into(), false, true),
        (
            "Source0: https://example.org/archive\n%include absent.inc\n".into(),
            true,
            true,
        ),
        ("".into(), false, false),
    ] {
        let source = format!("{}{}{}", &SPEC[..start], declarations, &SPEC[end..]);
        let _ = fs::remove_dir_all(dir.path().join(".ruyipack-stage"));
        fs::write(dir.path().join("ed.spec"), &source).unwrap();
        for policy in ["authoring", "submit"] {
            let output = run(
                dir.path(),
                &[
                    "check",
                    "--spec=ed.spec",
                    "--format",
                    "toml",
                    "--policy",
                    policy,
                ],
            );
            let report = super::support::machine_report(&output);
            let evidence = &report["evidence"];
            let blocks = policy == "submit";
            let expected_status = if blocks && violation {
                "fail"
            } else if blocks && unknown {
                "incomplete"
            } else {
                "pass"
            };
            assert_eq!(
                output.status.code(),
                Some(i32::from(blocks && (violation || unknown))),
                "{declarations}: {report}"
            );
            assert_eq!(evidence["status"].as_str(), Some(expected_status));
            assert_eq!(evidence["policy"].as_str(), Some(policy));
            assert_eq!(
                evidence
                    .get("source_uncertainty")
                    .is_some_and(toml::Value::is_str),
                unknown
            );
            assert_eq!(
                evidence["incomplete_reasons"],
                toml::Value::try_from(
                    &(if blocks && unknown {
                        toml::Value::Array(vec![toml::Value::from("unresolved-sources")])
                    } else {
                        toml::Value::Array(vec![])
                    })
                )
                .unwrap()
            );
            assert_eq!(
                evidence["not_checked"],
                toml::Value::Array(vec![
                    toml::Value::from("source-content"),
                    toml::Value::from("native-rpm"),
                    toml::Value::from("build")
                ])
            );
            let findings = report["findings"].as_array().unwrap();
            assert_eq!(
                findings.len(),
                usize::from(violation),
                "{declarations}: {report}"
            );
            if violation {
                assert_eq!(findings[0]["code"].as_str(), Some("RPK005"));
                assert_eq!(
                    findings[0]["severity"].as_str(),
                    Some(if blocks { "deny" } else { "warn" })
                );
                let span = &findings[0]["span"];
                assert!(
                    source[span["start_byte"].as_integer().unwrap() as usize
                        ..span["end_byte"].as_integer().unwrap() as usize]
                        .starts_with("Source0:")
                );
            }
        }
        let edited = run(
            dir.path(),
            &[
                "edit",
                "--spec=ed.spec",
                "--set",
                "package.version=2",
                "--diff",
            ],
        );
        assert!(edited.status.success(), "{declarations}: {edited:?}");
        assert!(String::from_utf8_lossy(&edited.stdout).contains("+Version:        2"));
        assert_file(dir.path().join("ed.spec"), &source);
    }
}

#[test]
fn source_definitions_are_recorded_without_claiming_native_evaluation() {
    let dir = tempfile::tempdir().unwrap();
    let source = SPEC.replace(
        "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
        "%{archive}",
    );
    fs::write(dir.path().join("ed.spec"), &source).unwrap();
    let definition = "archive https://example.org/archive";
    let output = run(
        dir.path(),
        &[
            "check",
            "--spec=ed.spec",
            "--policy",
            "submit",
            "--format",
            "toml",
            "-D",
            definition,
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let report = super::support::machine_report(&output);
    assert_eq!(
        report["evidence"]["defines"],
        toml::Value::Array(vec![toml::Value::from(definition)])
    );
    assert!(report["evidence"].get("source_uncertainty").is_none());
    assert_file(dir.path().join("ed.spec"), &source);
    let output = run(
        dir.path(),
        &[
            "check",
            "--spec=ed.spec",
            "--policy",
            "submit",
            "--format",
            "toml",
            "-D",
            "broken(",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report = super::support::machine_report(&output);
    assert_eq!(report["evidence"]["status"].as_str(), Some("incomplete"));
    assert!(
        report["evidence"]
            .get("source_uncertainty")
            .is_some_and(toml::Value::is_str)
    );
}
