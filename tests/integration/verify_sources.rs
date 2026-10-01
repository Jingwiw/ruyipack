// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Verification compares declarations with fresh bytes, including already-hashed URLs.

use super::{
    http::{Server, response},
    support::{assert_file, machine_report, success},
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
            .args(["source", "verify"])
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(&["--spec=input.spec", "--format", "toml"]);
    assert_eq!(output.status.code(), Some(1));
    let report = machine_report(&output);
    assert_eq!(
        report["input"]["sha256"].as_str(),
        Some((format!("{:x}", Sha256::digest(source.as_bytes()))).as_str())
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
        assert_eq!(report["sources"][n]["number"].as_integer(), Some(n as i64));
        assert_eq!(
            report["sources"][n]["status"].as_str(),
            Some(*expected),
            "{report}"
        );
    }
    assert_eq!(
        report["sources"][0]["declared_sha256"].as_str(),
        Some(hash.to_uppercase().as_str())
    );
    assert_eq!(
        report["sources"][1]["download"]["sha256"].as_str(),
        Some(hash.as_ref())
    );
    assert_eq!(
        report["sources"][2]["download"]["sha256"].as_str(),
        Some(hash.as_ref())
    );
    assert!(report["sources"][4].get("download").is_none());
    assert_eq!(report["sources"][4]["reason"].as_str(), Some("http-status"));
    assert_eq!(report["sources"][4]["http_status"].as_integer(), Some(404));
    assert_eq!(report["sources"][6]["reason"].as_str(), Some("resolution"));
    assert_eq!(report["sources"][6]["retryable"].as_bool(), Some(false));
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
    success(&run(&["--spec=input.spec"]));
    *mode.lock().unwrap() = "replaced";
    let changed = run(&["--spec=input.spec", "--format", "toml"]);
    assert_eq!(changed.status.code(), Some(1));
    assert_eq!(
        machine_report(&changed)["sources"][0]["status"].as_str(),
        Some("mismatch")
    );
    assert_file(&input, &matched);
    *mode.lock().unwrap() = "drift";
    let drift = run(&["--spec=input.spec", "--format", "toml"]);
    assert_eq!(
        machine_report(&drift)["error"]["code"].as_str(),
        Some("source-changed")
    );
    assert_file(&input, "# concurrent change\n");
    *mode.lock().unwrap() = "ok";
    let mut manifest: toml::Value =
        toml::from_str(include_str!("../../examples/ed/ed.toml")).unwrap();
    manifest["sources"]["0"]["url"] = format!("{}/match", server.url).into();
    manifest["sources"]["0"]["sha256"] = hash.into();
    manifest["sources"]
        .as_table_mut()
        .unwrap()
        .insert("1".into(), toml::toml! { path = "local:1.tar.gz" }.into());
    let original = toml::to_string(&manifest).unwrap();
    fs::write(root.join("ed.toml"), &original).unwrap();
    let output = run(&["--manifest=ed.toml", "--format", "toml"]);
    success(&output);
    assert_eq!(
        machine_report(&output)["sources"][0]["status"].as_str(),
        Some("match")
    );
    assert_eq!(
        machine_report(&output)["sources"][1]["status"].as_str(),
        Some("not-applicable")
    );
    assert_file(root.join("ed.toml"), &original);
    manifest["sources"]["0"]
        .as_table_mut()
        .unwrap()
        .remove("sha256");
    let missing = toml::to_string(&manifest).unwrap();
    fs::write(root.join("ed.toml"), &missing).unwrap();
    let output = run(&["--manifest=ed.toml", "--format", "toml"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        machine_report(&output)["sources"][0]["status"].as_str(),
        Some("missing")
    );
    assert_file(root.join("ed.toml"), &missing);
    fs::write(&input, "%include absent.inc\n").unwrap();
    let incomplete = run(&["--spec=input.spec", "--format", "toml"]);
    assert_eq!(incomplete.status.code(), Some(1));
    assert_eq!(
        machine_report(&incomplete)["error"]["code"].as_str(),
        Some("source-resolution")
    );
}
