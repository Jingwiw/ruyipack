// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Subprocess failures must never become a digest; native RPM is checked separately.

use super::support::{assert_file, command, json_line};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn source_hash_records_downloaded_bytes_and_refuses_unproven_results() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let spec = "Name: hash-probe\nVersion: 1\nRelease: 1\nSummary: hash\nLicense: MIT\nSource3: https://example.org/old\nSource: https://example.org/archive#/local.tar\n%description\nSource4: not a declaration\n%files\n";
    fs::write(root.join("input.spec"), spec).unwrap();
    for (name, body) in [
        (
            "rpmspec",
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'RPM version test'; exit; fi
printf '%s\n' "$@" > "$LOG/rpm-args"
if [ "$MODE" = diagnostic ]; then echo 'error: native failure' >&2; fi
for last do :; done
cat "$last"
"#,
        ),
        (
            "curl",
            r#"#!/bin/sh
printf '%s\n' "$@" > "$LOG/curl-args"
if [ "$MODE" = download-failure ]; then exit 22; fi
while [ "$1" != --output ]; do shift; done
printf '\000\377asset\n' > "$2"
if [ "$MODE" = source-changed ]; then echo changed >> "$LOG/input.spec"; fi
printf 'https://cdn.example.org/archive'
"#,
        ),
    ] {
        let path = root.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", root.display(), std::env::var("PATH").unwrap());
    let run = |mode: &str, trusted: bool| {
        let mut command = command();
        command
            .current_dir(root)
            .env("PATH", &path)
            .env("LOG", root)
            .env("MODE", mode)
            .args([
                "source-hash",
                "input.spec",
                "--source",
                "4",
                "--format",
                "json",
                "--define",
                "release 2; literal",
            ]);
        if trusted {
            command.arg("--trusted-spec");
        }
        command.output().unwrap()
    };
    let refused = run("", false);
    assert_eq!(refused.status.code(), Some(2));
    assert!(!root.join("rpm-args").exists());
    let output = run("", true);
    assert!(output.status.success(), "{output:?}");
    assert_file(root.join("input.spec"), spec);
    let report = json_line(&output);
    assert_eq!(
        report["sha256"],
        format!("{:x}", Sha256::digest(b"\0\xffasset\n"))
    );
    assert_eq!(report["bytes"], 8);
    assert_eq!(report["source"], 4);
    assert_eq!(
        report["resolved_url"],
        "https://example.org/archive#/local.tar"
    );
    assert_eq!(report["effective_url"], "https://cdn.example.org/archive");
    assert_eq!(report["native"]["defines"][0], "release 2; literal");
    assert!(
        fs::read_to_string(root.join("rpm-args"))
            .unwrap()
            .contains("--define\nrelease 2; literal\n")
    );
    let curl = fs::read_to_string(root.join("curl-args")).unwrap();
    assert!(curl.ends_with("--\nhttps://example.org/archive\n"));
    assert!(curl.contains("--proto-redir\n=http,https\n"));
    fs::remove_file(root.join("curl-args")).unwrap();
    let failed = |mode: &str, message: &str| {
        let output = run(mode, true);
        assert_eq!(output.status.code(), Some(1));
        let report = json_line(&output);
        assert_eq!(report["valid"], false);
        assert!(report["sha256"].is_null());
        assert!(report["error"].as_str().unwrap().contains(message));
    };
    failed("diagnostic", "native failure");
    assert!(!root.join("curl-args").exists());
    failed("download-failure", "curl download");
    failed("source-changed", "SPEC changed");
    for invalid in [
        "file:///etc/passwd",
        "https://user:secret@example.org/archive",
        "https://example.org/%{unknown}",
    ] {
        fs::write(
            root.join("input.spec"),
            spec.replace("https://example.org/archive#/local.tar", invalid),
        )
        .unwrap();
        fs::remove_file(root.join("curl-args")).ok();
        assert_eq!(run("", true).status.code(), Some(1));
        assert!(!root.join("curl-args").exists());
    }
}

#[test]
fn generation_and_edit_complete_digests_without_publishing_unchecked_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (name, body) in [
        (
            "rpmspec",
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'RPM version fixture'; exit; fi
echo native >> "$LOG/calls"
for last do :; done
version=$(sed -n 's/^Version: *//p' "$last")
sed "s/%{version}/$version/g" "$last"
"#,
        ),
        (
            "curl",
            r#"#!/bin/sh
echo download >> "$LOG/calls"
for last do :; done
while [ "$1" != --output ]; do shift; done
printf '%s' "$last" > "$2"
if [ "$MODE" = fail ]; then exit 22; fi
if [ "$MODE" = drift ]; then echo '# concurrent edit' >> "$LOG/input.spec"; fi
if [ "$MODE" = manifest-drift ]; then echo '# concurrent edit' >> "$LOG/ed.toml"; fi
printf '%s' "$last"
"#,
        ),
    ] {
        let path = root.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |args: &[&str], mode: &str| {
        command()
            .current_dir(root)
            .env(
                "PATH",
                format!("{}:{}", root.display(), std::env::var("PATH").unwrap()),
            )
            .env("LOG", root)
            .env("MODE", mode)
            .args(args)
            .output()
            .unwrap()
    };
    let source_url = "https://example.org/%{name}-%{version}.tar.gz#/renamed.tar.gz";
    let init = run(&["init", "ed"], "");
    assert!(init.status.success(), "{init:?}");
    assert!(!root.join("calls").exists());
    let mut manifest: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("ed.toml")).unwrap()).unwrap();
    assert_eq!(manifest["package"]["version"].as_str(), Some(""));
    assert!(manifest["sources"]["0"].get("sha256").is_none());
    let example: toml::Value = toml::from_str(include_str!("../../examples/ed/ed.toml")).unwrap();
    // Fill the handwritten scaffold; neither init flags nor a second TOML are needed.
    for section in ["spec", "package", "build", "build-requires"] {
        manifest
            .as_table_mut()
            .unwrap()
            .insert(section.into(), example[section].clone());
    }
    manifest["sources"]["0"]["url"] = source_url.into();
    let mut known = example["sources"]["0"].clone();
    known["url"] = "https://example.org/known".into();
    manifest["sources"]
        .as_table_mut()
        .unwrap()
        .insert("1".into(), known);
    manifest["sources"].as_table_mut().unwrap().insert(
        "2".into(),
        toml::Value::Table([("path".into(), "ed.conf".into())].into_iter().collect()),
    );
    let original = toml::to_string(&manifest).unwrap();
    fs::write(root.join("ed.toml"), &original).unwrap();
    let offline = run(&["gen", "ed", "--stdout"], "fail");
    assert!(offline.status.success(), "{offline:?}");
    assert!(String::from_utf8_lossy(&offline.stderr).contains("no sha256"));
    assert!(!root.join("calls").exists());
    for invalid in [
        original.replace(r#"version = "1.22.5""#, r#"version = """#),
        original.replace("GPL-3.0-or-later AND LGPL-2.1-or-later", "INVALID"),
        original.replace(
            "https://example.org/known",
            "https://example.org/%{unknown}",
        ),
    ] {
        assert_ne!(invalid, original);
        fs::write(root.join("ed.toml"), invalid).unwrap();
        let rejected = run(&["gen", "ed", "--hash-sources"], "");
        assert_eq!(rejected.status.code(), Some(1), "{rejected:?}");
        assert!(!root.join("calls").exists());
        assert!(!root.join("ed.spec").exists());
    }
    fs::write(root.join("ed.toml"), &original).unwrap();
    let mismatch = run(
        &["gen", "other", "--manifest", "ed.toml", "--hash-sources"],
        "",
    );
    assert_eq!(mismatch.status.code(), Some(1));
    assert!(!root.join("calls").exists());
    let failed = run(&["gen", "ed", "--hash-sources"], "fail");
    assert_eq!(failed.status.code(), Some(1));
    assert!(!root.join("ed.spec").exists());
    assert_file(root.join("ed.toml"), &original);
    let drifted = run(&["gen", "ed", "--hash-sources"], "manifest-drift");
    assert_eq!(drifted.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&drifted.stderr).contains("manifest changed"));
    assert!(!root.join("ed.spec").exists());
    assert_file(
        root.join("ed.toml"),
        &(original.clone() + "# concurrent edit\n"),
    );
    fs::write(root.join("ed.toml"), &original).unwrap();
    let digest = format!(
        "{:x}",
        Sha256::digest(b"https://example.org/ed-1.22.5.tar.gz")
    );
    let checked = run(
        &["gen", "ed", "--hash-sources", "--check", "--format", "json"],
        "",
    );
    assert!(checked.status.success(), "{checked:?}");
    let report = json_line(&checked);
    assert_eq!(report["valid"], true);
    assert_eq!(report["source_hashes"]["0"]["sha256"], digest);
    assert_eq!(
        report["source_hashes"]["0"]["resolved_url"],
        "https://example.org/ed-1.22.5.tar.gz#/renamed.tar.gz"
    );
    assert_eq!(
        report["manifest"]["sha256"],
        format!("{:x}", Sha256::digest(original.as_bytes()))
    );
    assert!(!root.join("ed.spec").exists());
    fs::remove_file(root.join("calls")).unwrap();
    let generated = run(&["gen", "ed", "--hash-sources"], "");
    assert!(generated.status.success(), "{generated:?}");
    let expected = String::from_utf8(offline.stdout).unwrap().replace(
        "#!RemoteAsset\n",
        &format!("#!RemoteAsset:  sha256:{digest}\n"),
    );
    assert_file(root.join("ed.spec"), &expected);
    assert_file(root.join("ed.toml"), &original);
    assert_file(root.join("calls"), "download\n");
    assert_eq!(
        fs::read_dir(root)
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "toml"))
            .count(),
        1
    );
    fs::remove_file(root.join("calls")).unwrap();
    manifest["sources"]["0"]
        .as_table_mut()
        .unwrap()
        .insert("sha256".into(), digest.into());
    fs::write(root.join("ed.toml"), toml::to_string(&manifest).unwrap()).unwrap();
    let known = run(&["gen", "ed", "--hash-sources", "--stdout"], "fail");
    assert!(known.status.success(), "{known:?}");
    assert_eq!(known.stdout, expected.as_bytes());
    assert!(!root.join("calls").exists());

    let spec = "Name: hash-probe\nVersion: 1\nRelease: 1\nSummary: Hash probe\nLicense: MIT\nURL: https://example.org\n#!RemoteAsset\nSource0: https://example.org/%{version}.tar.gz\n%description\nKeep this text.\n%files\n";
    let input = root.join("input.spec");
    fs::write(&input, spec).unwrap();
    let args = [
        "edit",
        "input.spec",
        "--set",
        "package.version=2",
        "--hash-source",
        "0",
        "--trusted-spec",
    ];
    let checked = run(
        &[args.as_slice(), &["--check", "--format", "json"]].concat(),
        "",
    );
    assert!(checked.status.success(), "{checked:?}");
    let report: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    let hashes = &report["files"][0]["source_hashes"];
    let digest = format!("{:x}", Sha256::digest(b"https://example.org/2.tar.gz"));
    assert_eq!(hashes["sources"]["0"]["sha256"], digest);
    assert_eq!(hashes["native"]["temporary_candidate"], true);
    assert_file(&input, spec);
    let preview = run(&[args.as_slice(), &["--diff"]].concat(), "");
    assert!(preview.status.success(), "{preview:?}");
    assert_file(&input, spec);
    let failed = run(&args, "fail");
    assert_eq!(failed.status.code(), Some(1));
    assert_file(&input, spec);
    let old_hash = format!("{:x}", Sha256::digest(spec.as_bytes()));
    let applied = run(
        &[
            args.as_slice(),
            &["--expect-sha256", &old_hash, "--format", "json"],
        ]
        .concat(),
        "",
    );
    assert!(applied.status.success(), "{applied:?}");
    let expected = spec.replace("Version: 1", "Version: 2").replace(
        "#!RemoteAsset\n",
        &format!("#!RemoteAsset:  sha256:{digest}\n"),
    );
    assert_file(&input, &expected);
    let receipt: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(receipt["outcomes"][0]["status"], "written");
    assert_eq!(
        receipt["outcomes"][0]["sha256"],
        format!("{:x}", Sha256::digest(expected.as_bytes()))
    );
    fs::remove_file(root.join("calls")).unwrap();
    let stale = run(
        &[
            "edit",
            "input.spec",
            "--expect-sha256",
            &old_hash,
            "--hash-source",
            "0",
            "--trusted-spec",
            "--check",
            "--format",
            "json",
        ],
        "",
    );
    assert_eq!(stale.status.code(), Some(1));
    assert!(!root.join("calls").exists());
    assert_file(&input, &expected);
    fs::write(
        &input,
        spec.replace("#!RemoteAsset\n", "#!RemoteAsset:  sha256:INVALID\n"),
    )
    .unwrap();
    let repaired = run(
        &["edit", "input.spec", "--hash-source", "0", "--trusted-spec"],
        "",
    );
    assert!(repaired.status.success(), "{repaired:?}");
    assert!(!fs::read_to_string(&input).unwrap().contains("INVALID"));
    fs::write(
        &input,
        spec.replace(
            "#!RemoteAsset\n",
            "#!RemoteAsset:  sha256:%{lua:rpm.define('archive_version 9')}\n",
        ),
    )
    .unwrap();
    fs::remove_file(root.join("calls")).unwrap();
    let rejected = run(&args, "");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("macro-bearing RemoteAsset"));
    assert!(!root.join("calls").exists());
    fs::write(&input, spec).unwrap();
    let prepared = run(
        &[
            "edit",
            "input.spec",
            "--hash-source",
            "0",
            "--trusted-spec",
            "--prepare",
            "drafts",
            "--format",
            "json",
        ],
        "",
    );
    assert!(prepared.status.success(), "{prepared:?}");
    let receipt: serde_json::Value = serde_json::from_slice(&prepared.stdout).unwrap();
    let draft = receipt["files"][0]["draft"].as_str().unwrap();
    let document: toml::Value = toml::from_str(&fs::read_to_string(draft).unwrap()).unwrap();
    assert_eq!(
        document["sources"]["0"]["sha256"].as_str(),
        Some(format!("{:x}", Sha256::digest(b"https://example.org/1.tar.gz")).as_str())
    );
    assert_file(&input, spec);
    let from = run(
        &[
            "edit",
            "--from",
            "drafts",
            "--hash-source",
            "0",
            "--trusted-spec",
            "--check",
            "--format",
            "json",
        ],
        "",
    );
    assert!(from.status.success(), "{from:?}");
    assert_file(&input, spec);
    let drifted = run(&args, "drift");
    assert_eq!(drifted.status.code(), Some(1));
    assert_file(&input, &(spec.to_owned() + "# concurrent edit\n"));
}
