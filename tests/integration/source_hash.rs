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
