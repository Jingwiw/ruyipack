// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Candidate, baseline, and partial-publication reports.

use super::{
    SPEC, assert_file, change_version, command, fixture, prepare, success, unchanged,
    version_source,
};
use std::fs;
use std::path::Path;

#[test]
fn invalid_toml_reports_its_file_line_and_column_in_check_json() {
    let directory = fixture(SPEC);
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    let path = drafts.join("ed.toml");
    fs::write(&path, "[package]\nversion = \"unterminated\n").unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .args(["--check", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert_eq!(report["files"][0]["error"]["code"], "invalid-draft");
    let error = report["files"][0]["error"]["message"].as_str().unwrap();
    assert!(error.contains("ed.toml:2:"), "{error}");
    assert_file(path, "[package]\nversion = \"unterminated\n");
    unchanged(directory.path());
}

#[test]
fn incomplete_batch_diagnostics_identify_each_candidate_without_publishing() {
    let directory = tempfile::tempdir().unwrap();
    let source = SPEC.replace(
        "GPL-3.0-or-later AND LGPL-2.1-or-later",
        "%{package_license}",
    );
    assert_ne!(source, SPEC);
    let names = ["first.spec", "second.spec"];
    for name in names {
        fs::write(directory.path().join(name), &source).unwrap();
    }
    let output = command(directory.path())
        .args(names)
        .args(["--set", "package.summary=Updated summary"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    for name in names {
        let path = fs::canonicalize(directory.path().join(name)).unwrap();
        let expected = format!(
            "{} (candidate): error: check incomplete because license expressions require RPM evaluation",
            path.display()
        );
        assert!(
            diagnostics.lines().any(|line| line == expected),
            "{diagnostics}"
        );
        assert_file(path, &source);
    }
}

#[test]
fn static_check_failure_blocks_even_forced_publication() {
    let directory = fixture(SPEC);
    let source = SPEC.replace("URL:            https://www.gnu.org/software/ed/\n", "");
    assert_ne!(source, SPEC);
    let path = directory.path().join("ed.spec");
    fs::write(&path, &source).unwrap();
    let checked = command(directory.path())
        .args([
            "ed.spec",
            "--set",
            "package.version=1.22.6",
            "--check",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    assert!(checked.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert_eq!(report["files"][0]["valid"], false);
    assert_eq!(
        report["files"][0]["report"]["findings"][0]["code"],
        "RPM015"
    );
    assert_eq!(report["files"][0]["introduced_static_blockers"], false);
    assert_eq!(
        report["files"][0]["baseline_report"]["evidence"]["status"],
        "fail"
    );
    assert!(
        report["files"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pre-existing")
    );
    assert_file(&path, &source);

    let target = directory.path().join("other.spec");
    fs::write(&target, "Keep this output\n").unwrap();
    for extra in [&[][..], &["--output", "other.spec", "--force"][..]] {
        let output = command(directory.path())
            .args(["ed.spec", "--set", "package.version=1.22.6"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("candidate failed static checks"));
        assert_file(&path, &source);
        assert_file(&target, "Keep this output\n");
    }
}

#[test]
fn json_reports_cover_check_prepare_apply_retry_and_partial_failure() {
    let directory = fixture(SPEC);
    for (file, exit) in [("ed.spec", 0), ("missing.spec", 1)] {
        let output = command(directory.path())
            .args([
                file,
                "--field",
                "package.version",
                "--check",
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert!(output.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["format_version"], 2);
        assert_eq!(report["scope"], "selected-edit-static");
        assert_eq!(report["valid"], exit == 0);
        if exit == 0 {
            assert_eq!(
                report["files"][0]["report"]["evidence"]["stage"],
                "spec-static"
            );
        } else {
            assert!(report["files"].as_array().unwrap().is_empty());
            assert!(report["error"]["code"].is_string());
            assert!(report["error"]["message"].is_string());
        }
    }
    unchanged(directory.path());
    let prepared = command(directory.path())
        .args([
            "ed.spec",
            "--field",
            "package.version",
            "--prepare",
            "drafts",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    success(&prepared);
    let receipt: serde_json::Value = serde_json::from_slice(&prepared.stdout).unwrap();
    let draft = Path::new(receipt["files"][0]["draft"].as_str().unwrap());
    change_version(draft, "2");
    let applied = command(directory.path())
        .args(["--from", "drafts", "--format", "json"])
        .output()
        .unwrap();
    success(&applied);
    let receipt: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(receipt["outcomes"][0]["status"], "written");
    assert_file(directory.path().join("ed.spec"), &version_source("2"));
    let repeated = command(directory.path())
        .args(["ed.spec", "--set", "package.version=2", "--format", "json"])
        .output()
        .unwrap();
    success(&repeated);
    let receipt: serde_json::Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(receipt["outcomes"][0]["status"], "unchanged");
    let stale = command(directory.path())
        .args(["--from", "drafts", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(1));
    let receipt: serde_json::Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(receipt["files"][0]["error"]["code"], "source-changed");
    assert_file(directory.path().join("ed.spec"), &version_source("2"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = directory.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("second.spec"), SPEC).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let partial = command(directory.path())
            .args([
                "ed.spec",
                "locked/second.spec",
                "--set",
                "package.version=3",
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(partial.status.code(), Some(1));
        assert!(partial.stderr.is_empty());
        let receipt: serde_json::Value = serde_json::from_slice(&partial.stdout).unwrap();
        let written = fs::canonicalize(directory.path().join("ed.spec")).unwrap();
        assert_eq!(receipt["written"], serde_json::json!([written]));
        assert_eq!(receipt["valid"], false);
        assert_file(written, &version_source("3"));
        assert_file(locked.join("second.spec"), SPEC);
    }
}

#[test]
fn upgrade_review_is_visible_without_changing_static_check_success() {
    let directory = fixture(SPEC);
    for (assignment, trigger) in [
        ("package.version=2", Some("package.version")),
        (
            "sources.0.url=https://example.org/new.tar.gz",
            Some("sources.0.url"),
        ),
        ("package.version=1.22.5", None),
        ("package.summary=Updated summary", None),
    ] {
        let output = command(directory.path())
            .args([
                "ed.spec", "--set", assignment, "--check", "--format", "json",
            ])
            .output()
            .unwrap();
        success(&output);
        assert!(output.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let file = &report["files"][0];
        assert_eq!(file["valid"], true);
        assert_eq!(file["report"]["evidence"]["status"], "pass");
        if let Some(trigger) = trigger {
            assert_eq!(file["review_triggers"], serde_json::json!([trigger]));
            assert_eq!(
                file["review_required"],
                serde_json::json!([
                    "source-content-and-digests",
                    "patch-applicability",
                    "native-build"
                ])
            );
        } else {
            assert_eq!(file["review_triggers"], serde_json::json!([]));
            assert_eq!(file["review_required"], serde_json::json!([]));
        }
        let preview = command(directory.path())
            .args(["ed.spec", "--set", assignment, "--stdout"])
            .output()
            .unwrap();
        success(&preview);
        assert_eq!(
            String::from_utf8_lossy(&preview.stderr).contains("review required"),
            trigger.is_some()
        );
        assert!(String::from_utf8_lossy(&preview.stdout).contains("sha256:56e107"));
    }
    unchanged(directory.path());
}

#[test]
fn edit_reports_bind_original_candidate_and_profile_without_inventing_a_path() {
    use sha2::{Digest, Sha256};
    let directory = fixture(SPEC);
    let source_path = directory.path().join("ed.spec").canonicalize().unwrap();
    let output = command(directory.path())
        .args([
            "ed.spec",
            "--set",
            "package.version=2",
            "--check",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    success(&output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let file = &value["files"][0];
    assert_eq!(file["source"], source_path.to_str().unwrap());
    assert_eq!(
        file["report"]["input"]["display_path"],
        source_path.to_str().unwrap()
    );
    assert_eq!(
        file["original_sha256"],
        format!("{:x}", Sha256::digest(SPEC.as_bytes()))
    );
    assert_eq!(
        file["report"]["input"]["sha256"],
        format!("{:x}", Sha256::digest(version_source("2").as_bytes()))
    );
    assert_eq!(file["report_subject"], "candidate");
    assert_eq!(file["profile"]["name"], "openruyi");
    assert_eq!(
        file["profile"]["sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../../../profiles/openruyi/profile.toml"))
        )
    );
    assert_eq!(file["report"]["evidence"]["stage"], "spec-static");
    assert!(file.get("environment").is_none());
    unchanged(directory.path());
}
