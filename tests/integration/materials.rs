// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

use super::support::{command, json_line, success};
use sha2::{Digest, Sha256};
use std::fs;

#[test]
fn inventory_is_offline_ordered_and_bound_to_actual_staged_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.spec");
    let materials = dir.path().join("SOURCES");
    fs::create_dir(&materials).unwrap();
    fs::write(materials.join("archive"), b"\0\xffbinary").unwrap();
    fs::write(materials.join("first.patch"), b"first").unwrap();
    fs::write(materials.join("second.patch"), b"second").unwrap();
    let hash = format!("{:x}", Sha256::digest(b"\0\xffbinary"));
    let spec = format!(
        "Name: probe\nVersion: 1\nRelease: 1\nSummary: Probe\nLicense: MIT\nURL: https://example.org\n#!RemoteAsset:  sha256:{hash}\nSource3: https://example.invalid/archive\n#!RemoteAsset\nSource: https://example.invalid/archive\nPatch20: nested/first.patch\nPatch0: second.patch\n"
    );
    fs::write(&input, &spec).unwrap();
    let run = || {
        command()
            .args(["check", "--materials"])
            .arg(&input)
            .arg("--source-dir")
            .arg(&materials)
            .args(["--format", "json"])
            .output()
            .unwrap()
    };
    let output = run();
    success(&output);
    let report = json_line(&output);
    let rows = report["materials"]["files"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["identity"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Source3", "Source4", "Patch20", "Patch0"]
    );
    assert_eq!(rows[0]["content"]["sha256"], hash);
    assert_eq!(rows[0]["content"]["size"], 8);
    assert_eq!(
        rows[2]["path"],
        materials
            .canonicalize()
            .unwrap()
            .join("first.patch")
            .to_str()
            .unwrap()
    );
    assert_eq!(rows[0]["expression"], "https://example.invalid/archive");
    assert_eq!(json_line(&run()), report);
    fs::write(materials.join("archive"), b"replaced").unwrap();
    let output = run();
    assert_eq!(output.status.code(), Some(1));
    let report = json_line(&output);
    assert_eq!(
        report["materials"]["files"][0]["error"]["code"],
        "digest-mismatch"
    );
    assert_eq!(
        report["materials"]["files"][0]["content"]["sha256"],
        format!("{:x}", Sha256::digest(b"replaced"))
    );
    assert_eq!(fs::read_to_string(&input).unwrap(), spec);
    assert_eq!(fs::read(materials.join("archive")).unwrap(), b"replaced");
}

#[test]
fn failures_are_complete_actionable_and_do_not_execute_macros() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.spec");
    fs::write(dir.path().join("same"), "present").unwrap();
    fs::create_dir(dir.path().join("directory")).unwrap();
    for (spec, code, row) in [
        (
            "Source0: absent\nPatch0: missing.patch\n",
            "missing-file",
            true,
        ),
        ("Source0: a/same\nPatch0: b/same\n", "name-collision", true),
        ("Source0: directory\n", "not-regular-file", true),
        ("Source0: ../\n", "invalid-filename", true),
        ("Source0: %{missing}\n", "unresolved", true),
        ("Source0: %(touch executed)\n", "unresolved", true),
        ("%include other.spec\nSource0: same\n", "unresolved", true),
        ("Patch: same\nPatch0: same\n", "material-resolution", false),
        (
            "%if %{unknown}\nPatch0: same\n%endif\n",
            "material-resolution",
            false,
        ),
        ("%patchlist\nsame\n", "material-resolution", false),
    ] {
        fs::write(&input, spec).unwrap();
        let output = command()
            .current_dir(dir.path())
            .args(["check", "--materials", "input.spec", "--format", "json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{spec}: {output:?}");
        let report = json_line(&output);
        assert_eq!(report["valid"], false);
        if row {
            for r in report["materials"]["files"].as_array().unwrap() {
                assert_eq!(r["error"]["code"], code, "{spec}: {report}");
            }
            assert!(!report["materials"]["files"].as_array().unwrap().is_empty());
        } else {
            assert_eq!(
                report["materials"]["error"]["code"], code,
                "{spec}: {report}"
            );
        }
        assert_eq!(fs::read_to_string(&input).unwrap(), spec);
        assert!(!dir.path().join("executed").exists());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("same", dir.path().join("link")).unwrap();
        fs::write(&input, "Source0: link\n").unwrap();
        let output = command()
            .args(["check", "--materials"])
            .arg(&input)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert_eq!(
            json_line(&output)["materials"]["files"][0]["error"]["code"],
            "not-regular-file"
        );
    }
}

#[test]
fn manifest_and_generated_spec_inventory_agree_without_requiring_native_rpm() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.toml");
    let manifest = include_str!("../native/rpk-native.toml");
    fs::write(&input, manifest).unwrap();
    for (i, name) in [
        "rpk-native-1.tar.gz",
        "rpk-native.conf",
        "2000-first.patch",
        "0001-second.patch",
    ]
    .iter()
    .enumerate()
    {
        fs::write(dir.path().join(name), format!("material {i}")).unwrap();
    }
    let generated = command()
        .args(["gen", "rpk-native", "--offline", "--stdout", "--manifest"])
        .arg(&input)
        .output()
        .unwrap();
    success(&generated);
    let spec = dir.path().join("input.spec");
    fs::write(&spec, &generated.stdout).unwrap();
    let output = command()
        .args(["check", "--materials", "--manifest"])
        .arg(&input)
        .args(["--format", "json"])
        .output()
        .unwrap();
    success(&output);
    let report = json_line(&output);
    let output = command()
        .args(["check", "--materials"])
        .arg(&spec)
        .args(["--format", "json"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        report["materials"]["files"],
        json_line(&output)["materials"]["files"]
    );
    assert_eq!(
        report["input"]["sha256"],
        format!("{:x}", Sha256::digest(manifest.as_bytes()))
    );
    assert_eq!(
        report["generated_spec_sha256"],
        format!("{:x}", Sha256::digest(&generated.stdout))
    );
    let malformed = dir.path().join("malformed.toml");
    fs::write(&malformed, "[package]\n").unwrap();
    let invalid = command()
        .args(["check", "--manifest"])
        .arg(&malformed)
        .args(["--materials", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    assert_eq!(json_line(&invalid)["error"]["code"], "invalid-manifest");
    assert_eq!(fs::read_to_string(&input).unwrap(), manifest);
    // Macro definitions select real branches; unknown branches never count as checked.
    fs::write(
        &spec,
        "Name: probe\nVersion: 1\nRelease: 1\nSummary: Probe\nLicense: MIT\nURL: https://example.org\n%if 0%{?use_patch}\nPatch7: 0001-second.patch\n%endif\n",
    )
    .unwrap();
    let output = command()
        .args(["check", "--materials"])
        .arg(&spec)
        .args(["-D", "use_patch 1", "--format", "json"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        json_line(&output)["materials"]["files"][0]["identity"],
        "Patch7"
    );
}

#[test]
fn material_evidence_composes_with_static_policy_without_changing_it() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.spec");
    let prefix = "Name: probe\nVersion: 1\nRelease: 1\nSummary: Probe\nLicense: MIT\nURL: https://example.org\n";
    let run = |extra: &[&str]| {
        command()
            .args(["check", "--materials"])
            .arg(&input)
            .args(["--format", "json"])
            .args(extra)
            .output()
            .unwrap()
    };
    for row in include_str!("../fixtures/material-filenames.tsv")
        .lines()
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let (expression, name) = row.split_once('\t').unwrap();
        fs::write(dir.path().join(name), b"").unwrap();
        fs::write(
            &input,
            format!("{prefix}#!RemoteAsset\nSource0: {expression}\nPatch0: {expression}\n"),
        )
        .unwrap();
        let output = run(&[]);
        // URL policy can reject a local RemoteAsset marker; material facts remain useful.
        let report = json_line(&output);
        assert_eq!(report["materials"]["valid"], true, "{expression}: {report}");
        for entry in report["materials"]["files"].as_array().unwrap() {
            assert_eq!(
                entry["path"],
                dir.path()
                    .canonicalize()
                    .unwrap()
                    .join(name)
                    .to_str()
                    .unwrap()
            );
            assert_eq!(
                entry["content"]["sha256"],
                format!("{:x}", Sha256::digest(b""))
            );
        }
    }
    fs::write(dir.path().join("archive"), "bytes").unwrap();
    fs::write(
        &input,
        format!("{prefix}#!RemoteAsset\nSource0: https://example.invalid/archive\n"),
    )
    .unwrap();
    success(&run(&[])); // Missing digest is still an authoring warning, not invented authentication.
    let submit = run(&["--policy", "submit"]);
    assert_eq!(submit.status.code(), Some(1));
    let report = json_line(&submit);
    assert_eq!(report["valid"], false);
    assert_eq!(report["materials"]["valid"], true);
    assert_eq!(report["evidence"]["status"], "fail");
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "RPK005")
    );
    fs::write(&input, "Source0: archive\n").unwrap();
    let incomplete = run(&[]);
    assert_eq!(incomplete.status.code(), Some(1));
    assert_eq!(json_line(&incomplete)["materials"]["valid"], true);
    let invalid = command()
        .args(["check", "--source-dir"])
        .arg(dir.path())
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}
