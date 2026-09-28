// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Verification compares declarations with fresh bytes, including already-hashed URLs.

use super::{
    http::{Server, response},
    support::{assert_file, json_line, success},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    sync::{Arc, Mutex},
};

#[test]
fn verification_distinguishes_missing_mismatch_and_uncertainty_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_owned();
    let mode = Arc::new(Mutex::new("ok"));
    let current = mode.clone();
    let changed = root.join("input.spec");
    let server = Server::new(true, move |path| {
        let mode = *current.lock().unwrap();
        if mode == "drift" {
            fs::write(&changed, "# concurrent change\n").unwrap();
        }
        if path == "/fail" {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
        } else {
            response(if mode == "replaced" {
                b"changed upstream"
            } else {
                b"archive"
            })
        }
    });
    let hash = format!("{:x}", Sha256::digest(b"archive"));
    let source = format!(
        "Name: probe\nVersion: 1\n%global host {}\n#!RemoteAsset:  sha256:{}\nSource0: %{{host}}/match\n#!RemoteAsset:  sha256:{}\nSource1: %{{host}}/mismatch\nSource2: %{{host}}/missing\nSource3: local.tar\nSource4: %{{host}}/fail\n#!RemoteAsset:  sha256:{}\nSource5: %{{host}}/later\nSource6: %{{unknown}}/archive\n",
        server.url,
        hash.to_uppercase(),
        "a".repeat(64),
        hash
    );
    let input = root.join("input.spec");
    fs::write(&input, &source).unwrap();
    let run = |args: &[&str]| {
        server
            .command()
            .current_dir(&root)
            .arg("verify-sources")
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(&["input.spec", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_line(&output);
    assert_eq!(
        report["input"]["sha256"],
        format!("{:x}", Sha256::digest(source.as_bytes()))
    );
    for (n, expected) in [
        "match",
        "mismatch",
        "missing",
        "not-applicable",
        "error",
        "match",
        "unresolved",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            report["sources"][n.to_string()]["status"],
            *expected,
            "{report}"
        );
    }
    assert_eq!(
        report["sources"]["0"]["declared_sha256"],
        hash.to_uppercase()
    );
    assert_eq!(report["sources"]["1"]["download"]["sha256"], hash);
    assert_eq!(report["sources"]["2"]["download"]["sha256"], hash);
    assert!(report["sources"]["4"]["download"].is_null());
    assert_eq!(report["sources"]["4"]["reason"], "http-status");
    assert_eq!(report["sources"]["4"]["http_status"], 404);
    assert_eq!(report["sources"]["6"]["reason"], "resolution");
    assert_eq!(report["sources"]["6"]["retryable"], false);
    assert_file(&input, &source);
    assert_eq!(
        *server.calls.lock().unwrap(),
        ["/match", "/mismatch", "/missing", "/fail", "/later"]
    );
    let matched = source
        .split("Source1:")
        .next()
        .unwrap()
        .rsplit_once("#!RemoteAsset:")
        .unwrap()
        .0
        .to_owned();
    fs::write(&input, &matched).unwrap();
    success(&run(&["input.spec"]));
    *mode.lock().unwrap() = "replaced";
    let changed = run(&["input.spec", "--format", "json"]);
    assert_eq!(changed.status.code(), Some(1));
    assert_eq!(json_line(&changed)["sources"]["0"]["status"], "mismatch");
    assert_file(&input, &matched);
    *mode.lock().unwrap() = "drift";
    let drift = run(&["input.spec", "--format", "json"]);
    assert_eq!(json_line(&drift)["error"]["code"], "source-changed");
    assert_file(&input, "# concurrent change\n");
    *mode.lock().unwrap() = "ok";
    let mut manifest: toml::Value =
        toml::from_str(include_str!("../../examples/ed/ed.toml")).unwrap();
    manifest["sources"]["0"]["url"] = format!("{}/match", server.url).into();
    manifest["sources"]["0"]["sha256"] = hash.into();
    let original = toml::to_string(&manifest).unwrap();
    fs::write(root.join("ed.toml"), &original).unwrap();
    let output = run(&["--manifest", "ed.toml", "--format", "json"]);
    success(&output);
    assert_eq!(json_line(&output)["sources"]["0"]["status"], "match");
    assert_file(root.join("ed.toml"), &original);
    manifest["sources"]["0"]
        .as_table_mut()
        .unwrap()
        .remove("sha256");
    let missing = toml::to_string(&manifest).unwrap();
    fs::write(root.join("ed.toml"), &missing).unwrap();
    let output = run(&["--manifest", "ed.toml", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_line(&output)["sources"]["0"]["status"], "missing");
    assert_file(root.join("ed.toml"), &missing);
    fs::write(&input, "%include absent.inc\n").unwrap();
    let incomplete = run(&["input.spec", "--format", "json"]);
    assert_eq!(incomplete.status.code(), Some(1));
    assert_eq!(json_line(&incomplete)["error"]["code"], "source-resolution");
}
