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

#[test]
fn invalid_toml_reports_its_file_line_and_column_in_check_toml() {
    let directory = fixture(SPEC);
    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    let path = drafts.join("ed.toml");
    fs::write(&path, "[package]\nversion = \"unterminated\n").unwrap();
    let output = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .args(["--check", "--format", "toml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report = super::support::machine_report(&output);
    assert!(report.get("valid").is_none());
    assert_eq!(
        report["files"][0]["error"]["code"].as_str(),
        Some("invalid-draft")
    );
    let error = report["files"][0]["error"]["message"].as_str().unwrap();
    assert!(error.contains("ed.toml:2:24"), "{error}");
    assert_file(path, "[package]\nversion = \"unterminated\n");
    let preview = command(directory.path())
        .arg("--from")
        .arg(&drafts)
        .arg("--diff")
        .output()
        .unwrap();
    assert_eq!(preview.status.code(), Some(1));
    assert!(
        preview.stdout.is_empty(),
        "an unconstructable candidate must not produce a diff"
    );
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
    for diff in [false, true] {
        let output = command(directory.path())
            .args(names.into_iter().flat_map(|name| ["--spec", name]))
            .args(diff.then_some("--diff"))
            .args(["--set", "package.summary=Updated summary", "--check"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        if diff {
            let text = String::from_utf8_lossy(&output.stdout);
            assert_eq!(
                text.matches("+Summary:        Updated summary").count(),
                2,
                "{text}"
            );
        }
        if !diff {
            assert!(output.stdout.is_empty(), "{output:?}");
        }
        let diagnostics = String::from_utf8(output.stderr).unwrap();
        let starts = names.map(|name| {
            diagnostics
                .find(&format!("[INFO] {name}: candidate static blockers:"))
                .unwrap_or_else(|| panic!("missing {name} report context: {diagnostics}"))
        });
        assert!(starts[0] < starts[1], "{diagnostics}");
        let line = source
            .lines()
            .position(|line| line.starts_with("License:"))
            .unwrap()
            + 1;
        let issue = format!("[WARN] spec[{line}:1] [RPK001]: inherited 1 issue(s)");
        let incomplete =
            "[ERROR] check incomplete because license expressions require RPM evaluation";
        assert_eq!(diagnostics.matches(&issue).count(), 2, "{diagnostics}");
        assert_eq!(diagnostics.matches(incomplete).count(), 2, "{diagnostics}");
        for (index, name) in names.into_iter().enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(diagnostics.len());
            let section = &diagnostics[starts[index]..end];
            assert_eq!(section.matches(&issue).count(), 1, "{section}");
            assert_eq!(section.matches(incomplete).count(), 1, "{section}");
            assert!(
                section.contains(&format!("[ERROR] {name}: not admissible")),
                "{section}"
            );
            assert_file(directory.path().join(name), &source);
        }
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
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--check",
            "--format",
            "toml",
        ])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    assert!(checked.stderr.is_empty());
    let report = super::support::machine_report(&checked);
    assert_eq!(report["valid"].as_bool(), Some(false));
    assert_eq!(report["files"][0]["valid"].as_bool(), Some(false));
    assert_eq!(
        report["files"][0]["report"]["findings"][0]["code"].as_str(),
        Some("RPM015")
    );
    assert_eq!(
        report["files"][0]["introduced_static_blockers"].as_bool(),
        Some(false)
    );
    assert_eq!(
        report["files"][0]["baseline_report"]["evidence"]["status"].as_str(),
        Some("fail")
    );
    assert!(
        report["files"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pre-existing")
    );
    assert_file(&path, &source);

    let preview = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--diff",
            "--check",
        ])
        .output()
        .unwrap();
    assert_eq!(preview.status.code(), Some(1), "{preview:?}");
    assert!(String::from_utf8_lossy(&preview.stdout).contains("+Version:        1.22.6"));
    assert_eq!(
        String::from_utf8_lossy(&preview.stderr)
            .matches("spec is missing the URL: tag")
            .count(),
        1
    );
    assert_file(&path, &source);

    let target = directory.path().join("other.spec");
    fs::write(&target, "Keep this output\n").unwrap();
    for extra in [
        &[][..],
        &["--stdout"][..],
        &["--output", "other.spec", "--force"][..],
    ] {
        let output = command(directory.path())
            .args([
                "--spec=ed.spec",
                "--set",
                "package.version=1.22.6",
                "--apply",
            ])
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
fn toml_reports_cover_check_prepare_apply_retry_and_partial_failure() {
    let directory = fixture(SPEC);
    for (file, exit) in [("ed.spec", 0), ("missing.spec", 1)] {
        let output = command(directory.path())
            .args(["--spec", file, "--check", "--format", "toml"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit), "{output:?}");
        assert!(output.stderr.is_empty());
        let report = super::support::machine_report(&output);
        assert_eq!(report["format_version"].as_integer(), Some(4));
        assert_eq!(report["scope"].as_str(), Some("edit-stage"));
        assert_eq!(report["success"].as_bool(), Some(exit == 0));
        if exit == 0 {
            assert_eq!(report["valid"].as_bool(), Some(true));
            assert_eq!(report["files"][0]["state"].as_str(), Some("candidate"));
            assert_eq!(
                report["files"][0]["report"]["evidence"]["stage"].as_str(),
                Some("spec-static")
            );
        } else {
            assert!(report.get("valid").is_none());
            assert!(report["files"].as_array().unwrap().is_empty());
            assert!(report["error"]["code"].is_str());
            assert!(report["error"]["message"].is_str());
        }
    }
    unchanged(directory.path());

    // A failed publication retains its stage, so use an independent transaction.
    let failed_directory = fixture(SPEC);
    fs::create_dir(failed_directory.path().join("directory-target")).unwrap();
    let output = command(failed_directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=2",
            "--apply",
            "--output",
            "directory-target",
            "--format",
            "toml",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let receipt = super::support::machine_report(&output);
    assert_eq!(receipt["error"]["code"].as_str(), Some("operation-failed"));
    assert_eq!(receipt["error"]["stage"].as_str(), Some("publication"));
    assert_eq!(receipt["error"]["reason"].as_str(), Some("read-failed"));
    assert_eq!(receipt["error"]["io_kind"].as_str(), Some("is-a-directory"));
    assert!(receipt["error"]["path"].is_str());
    unchanged(failed_directory.path());

    let drafts = prepare(directory.path(), &["ed.spec"], &["package.version"]);
    change_version(&drafts.join("ed.toml"), "2");
    let applied = command(directory.path())
        .args(["--from", "drafts", "--apply", "--format", "toml"])
        .output()
        .unwrap();
    success(&applied);
    let receipt = super::support::machine_report(&applied);
    assert_eq!(receipt["outcomes"][0]["status"].as_str(), Some("written"));
    assert_file(directory.path().join("ed.spec"), &version_source("2"));
    let repeated = command(directory.path())
        .args(["--from", "drafts", "--apply", "--format", "toml"])
        .output()
        .unwrap();
    success(&repeated);
    let receipt = super::support::machine_report(&repeated);
    assert_eq!(receipt["outcomes"][0]["status"].as_str(), Some("unchanged"));
    // Successful application rebases the stage; only a later external change is stale.
    fs::write(directory.path().join("ed.spec"), version_source("3")).unwrap();
    let stale = command(directory.path())
        .args(["--from", "drafts", "--apply", "--format", "toml"])
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(1));
    let receipt = super::support::machine_report(&stale);
    assert_eq!(
        receipt["files"][0]["error"]["code"].as_str(),
        Some("source-changed")
    );
    assert_file(directory.path().join("ed.spec"), &version_source("3"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = fixture(SPEC);
        let locked = directory.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("second.spec"), SPEC).unwrap();
        let drafts = prepare(
            directory.path(),
            &["ed.spec", "locked/second.spec"],
            &["package.version"],
        );
        change_version(&drafts.join("ed.toml"), "3");
        change_version(&drafts.join("second.toml"), "3");
        // Prepare before locking the destination, isolating publication failure.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let partial = command(directory.path())
            .args(["--from", "drafts", "--apply", "--format", "toml"])
            .output()
            .unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(partial.status.code(), Some(1));
        assert!(partial.stderr.is_empty());
        let receipt = super::support::machine_report(&partial);
        let written = fs::canonicalize(directory.path().join("ed.spec")).unwrap();
        assert_eq!(
            receipt["written"]
                .as_array()
                .unwrap()
                .iter()
                .map(|path| path.as_str().unwrap())
                .collect::<Vec<_>>(),
            [written.to_str().unwrap()]
        );
        assert_eq!(receipt["success"].as_bool(), Some(false));
        assert_eq!(receipt["valid"].as_bool(), Some(true));
        assert_eq!(receipt["error"]["code"].as_str(), Some("operation-failed"));
        assert_eq!(receipt["error"]["stage"].as_str(), Some("publication"));
        assert_eq!(receipt["error"]["reason"].as_str(), Some("write-failed"));
        assert_eq!(
            receipt["error"]["io_kind"].as_str(),
            Some("permission-denied")
        );
        assert_eq!(
            receipt["error"]["path"].as_str(),
            Some(
                fs::canonicalize(locked.join("second.spec"))
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        );
        assert_file(written, &version_source("3"));
        assert_file(locked.join("second.spec"), SPEC);

        use std::os::unix::ffi::OsStringExt;
        let directory = fixture(SPEC);
        let missing = directory
            .path()
            .join(std::ffi::OsString::from_vec(b"missing-\xff".to_vec()));
        let failed = command(directory.path())
            .args([
                "--spec=ed.spec",
                "--set",
                "package.version=4",
                "--apply",
                "--format",
                "toml",
                "--output",
            ])
            .arg(missing.join("out.spec"))
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(1));
        assert!(failed.stderr.is_empty());
        let report = super::support::machine_report(&failed);
        assert_eq!(
            report["error"]["reason"].as_str(),
            Some("read-failed"),
            "{report}"
        );
        assert_eq!(
            report["error"]["path"].as_str(),
            Some(missing.to_string_lossy().as_ref())
        );
        unchanged(directory.path());
    }
}

#[test]
fn upgrade_review_is_visible_without_changing_static_check_success() {
    for (assignment, trigger) in [
        ("package.version=2", Some("package.version")),
        (
            "sources.0.url=https://example.org/new.tar.gz",
            Some("sources.0.url"),
        ),
        ("package.version=1.22.5", None),
        ("package.summary=Updated summary", None),
    ] {
        let directory = fixture(SPEC);
        let output = command(directory.path())
            .args([
                "--spec=ed.spec",
                "--set",
                assignment,
                "--check",
                "--format",
                "toml",
            ])
            .output()
            .unwrap();
        success(&output);
        assert!(output.stderr.is_empty());
        let report = super::support::machine_report(&output);
        let file = &report["files"][0];
        assert_eq!(file["valid"].as_bool(), Some(true));
        assert_eq!(file["report"]["evidence"]["status"].as_str(), Some("pass"));
        if let Some(trigger) = trigger {
            assert_eq!(
                file["review_triggers"],
                toml::Value::Array(vec![toml::Value::from(trigger)])
            );
            assert_eq!(
                file["review_required"],
                toml::Value::Array(vec![
                    toml::Value::from("source-content-and-digests"),
                    toml::Value::from("patch-applicability"),
                    toml::Value::from("native-build")
                ])
            );
        } else {
            assert_eq!(file["review_triggers"], toml::Value::Array(vec![]));
            assert_eq!(file["review_required"], toml::Value::Array(vec![]));
        }
        let preview = command(directory.path())
            .args(["--spec=ed.spec", "--set", assignment, "--stdout"])
            .output()
            .unwrap();
        success(&preview);
        assert_eq!(
            String::from_utf8_lossy(&preview.stderr).contains("review required"),
            trigger.is_some()
        );
        assert!(String::from_utf8_lossy(&preview.stdout).contains("sha256:56e107"));
        unchanged(directory.path());
    }
}

#[test]
fn edit_reports_bind_original_candidate_and_profile_without_inventing_a_path() {
    use sha2::{Digest, Sha256};
    let directory = fixture(SPEC);
    let source_path = directory.path().join("ed.spec").canonicalize().unwrap();
    let output = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set",
            "package.version=2",
            "--check",
            "--format",
            "toml",
        ])
        .output()
        .unwrap();
    success(&output);
    let value = super::support::machine_report(&output);
    let file = &value["files"][0];
    assert_eq!(file["source"].as_str(), Some(source_path.to_str().unwrap()));
    assert_eq!(
        file["report"]["input"]["display_path"].as_str(),
        Some(source_path.to_str().unwrap())
    );
    assert_eq!(
        file["original_sha256"].as_str(),
        Some(format!("{:x}", Sha256::digest(SPEC.as_bytes())).as_str())
    );
    assert_eq!(
        file["report"]["input"]["sha256"].as_str(),
        Some((format!("{:x}", Sha256::digest(version_source("2").as_bytes()))).as_str())
    );
    assert_eq!(file["profile"]["name"].as_str(), Some("openruyi"));
    assert_eq!(
        file["profile"]["sha256"].as_str(),
        Some(
            (format!(
                "{:x}",
                Sha256::digest(include_bytes!("../../../profiles/openruyi/profile.toml"))
            ))
            .as_str()
        )
    );
    assert_eq!(
        file["report"]["evidence"]["stage"].as_str(),
        Some("spec-static")
    );
    assert!(file.get("environment").is_none());
    unchanged(directory.path());
}

#[cfg(unix)]
#[test]
fn non_utf8_publication_reports_real_filesystem_outcomes() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let directory = fixture(SPEC);
    let target = directory
        .path()
        .canonicalize()
        .unwrap()
        .join(OsStr::from_bytes(b"review-\xff.spec"));
    let filename_supported = match fs::write(&target, "filename probe\n") {
        Ok(()) => {
            fs::remove_file(&target).unwrap();
            true
        }
        Err(error) => {
            let confirmed_filename_rejection = error.kind() == std::io::ErrorKind::InvalidFilename;
            #[cfg(target_os = "macos")]
            let confirmed_filename_rejection =
                confirmed_filename_rejection || error.raw_os_error() == Some(92); // EILSEQ
            assert!(
                confirmed_filename_rejection,
                "unexpected filename probe failure: {error}"
            );
            false
        }
    };
    let output = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set=package.version=2",
            "--apply",
            "--output",
        ])
        .arg(&target)
        .args(["--format", "toml"])
        .output()
        .unwrap();
    let report = super::support::machine_report(&output);
    if filename_supported {
        success(&output);
        assert_eq!(report["success"].as_bool(), Some(true));
        assert_eq!(report["outcomes"][0]["status"].as_str(), Some("written"));
        assert_eq!(
            report["outcomes"][0]["path"].as_str(),
            Some(target.to_string_lossy().as_ref())
        );
        assert_file(&target, &version_source("2"));
    } else {
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(report["success"].as_bool(), Some(false));
        assert_eq!(report["error"]["stage"].as_str(), Some("publication"));
        assert_eq!(report["error"]["reason"].as_str(), Some("write-failed"));
        assert_eq!(
            report["error"]["path"].as_str(),
            Some(target.to_string_lossy().as_ref())
        );
        assert!(
            fs::read_dir(directory.path())
                .unwrap()
                .all(|entry| entry.unwrap().file_name() != target.file_name().unwrap())
        );
    }
    assert_file(directory.path().join("ed.spec"), SPEC);
}
