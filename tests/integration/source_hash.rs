// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Actual loopback downloads exercise completion, failure and stale-input guards.

use super::{
    http::{Server, response},
    support::{assert_file, json_line, success},
};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    fs,
    sync::{Arc, Mutex},
};

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn streaming_hashes_validate_tls_redirects_and_complete_response_bodies() {
    let attempts = Cell::new(0);
    let redirects = Cell::new(0);
    let server = Server::new(true, move |path| {
        if path == "/retry" || path == "/busy" {
            if path == "/retry" {
                attempts.set(attempts.get() + 1);
            }
            if path == "/busy" || attempts.get() == 1 {
                return b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec();
            }
        }
        // Terminate a broken redirect loop in the fixture too, so a regression fails promptly.
        if path == "/loop" {
            redirects.set(redirects.get() + 1);
            if redirects.get() > 11 {
                return response(b"redirect limit was not enforced");
            }
        }
        match path {
        "/redirect" => b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /asset\r\nContent-Length: 0\r\n\r\n".to_vec(),
        "/loop" => b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n".to_vec(),
        "/bad-redirect" => b"HTTP/1.1 302 Found\r\nLocation: https://user:secret@localhost/archive\r\nContent-Length: 0\r\n\r\n".to_vec(),
        "/downgrade" => b"HTTP/1.1 302 Found\r\nLocation: http://localhost/archive\r\nContent-Length: 0\r\n\r\n".to_vec(),
        "/truncated" => b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort".to_vec(),
        "/partial" => b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\n\r\npart".to_vec(),
        "/fail" => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
        // Deliberately not a valid gzip stream: content decoding must be disabled.
        "/encoding" => b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 4\r\n\r\nraw!".to_vec(),
        _ => response(b"\0\xffasset\n"),
    }
    });
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.spec");
    let source = format!(
        "Name: probe\nVersion: 1\n%global route redirect\nSource3: local.tar\nSource: {}/%{{route}}#/renamed.tar\n%description\nProbe\n",
        server.url
    );
    let run = || {
        server
            .command()
            .args(["source-hash"])
            .arg(&input)
            .args(["--source", "4", "--format", "json"])
            .output()
            .unwrap()
    };
    fs::write(&input, &source).unwrap();
    let output = run();
    success(&output);
    let report = json_line(&output);
    assert_eq!(report["sha256"], sha(b"\0\xffasset\n"));
    assert_eq!(report["bytes"], 8);
    assert_eq!(report["effective_url"], format!("{}/asset", server.url));
    assert_eq!(*server.calls.lock().unwrap(), ["/redirect", "/asset"]);
    assert_file(&input, &source);
    for (path, reason) in [
        ("fail", "404"),
        ("busy", "503"),
        ("truncated", "download body"),
        ("partial", "206"),
        ("loop", "redirects"),
        ("bad-redirect", "credentials"),
        ("downgrade", "downgrade"),
    ] {
        fs::write(
            &input,
            source.replace("route redirect", &format!("route {path}")),
        )
        .unwrap();
        let output = run();
        assert_eq!(output.status.code(), Some(1), "{path}: {output:?}");
        let report = json_line(&output);
        assert!(report["sha256"].is_null());
        assert!(
            report["error"]["message"]
                .as_str()
                .unwrap()
                .contains(reason),
            "{report}"
        );
        assert!(!report.to_string().contains("secret"));
    }
    let calls = server.calls.lock().unwrap();
    for (path, count) in [
        ("/busy", 2),
        ("/fail", 1),
        ("/truncated", 2),
        ("/loop", 11),
        ("/bad-redirect", 1),
        ("/downgrade", 1),
    ] {
        assert_eq!(calls.iter().filter(|p| *p == path).count(), count, "{path}");
    }
    drop(calls);
    // A retry starts a fresh digest; a failed attempt never returns partial bytes.
    fs::write(&input, source.replace("route redirect", "route retry")).unwrap();
    let output = run();
    success(&output);
    assert_eq!(json_line(&output)["sha256"], sha(b"\0\xffasset\n"));
    assert_eq!(
        server
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == "/retry")
            .count(),
        2
    );
    fs::write(&input, source.replace("route redirect", "route encoding")).unwrap();
    let output = run();
    success(&output);
    assert_eq!(json_line(&output)["sha256"], sha(b"raw!"));
    let rejected = server
        .command()
        .env_remove("SSL_CERT_FILE")
        .arg("source-hash")
        .arg(&input)
        .args(["--source", "4", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(1)); // Never silently accept an untrusted TLS peer.
}

#[test]
fn generation_completes_only_missing_hashes_and_never_publishes_stale_input() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_owned();
    let input = root.join("ed.toml");
    let mode = Arc::new(Mutex::new("ok"));
    let current = mode.clone();
    let changed = input.clone();
    let server = Server::new(true, move |path| {
        let mode = *current.lock().unwrap();
        if mode.starts_with("drift") {
            fs::write(&changed, "# concurrent change\n").unwrap();
        }
        if mode.ends_with("fail") || path == "/fail" {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
        } else {
            response(path.as_bytes())
        }
    });
    let mut manifest: toml::Value =
        toml::from_str(include_str!("../../examples/ed/ed.toml")).unwrap();
    manifest["sources"]["0"]["url"] =
        format!("{}/ed-%{{version}}.tar.lz#/renamed.tar", server.url).into();
    manifest["sources"]["0"]
        .as_table_mut()
        .unwrap()
        .remove("sha256");
    let original = toml::to_string(&manifest).unwrap();
    fs::write(&input, &original).unwrap();
    let run = |extra: &[&str]| {
        server
            .command()
            .current_dir(&root)
            .args(["gen", "ed"])
            .args(extra)
            .output()
            .unwrap()
    };
    success(&run(&["--offline", "--stdout"]));
    assert!(server.calls.lock().unwrap().is_empty());
    let output = run(&["--check", "--format", "json"]);
    success(&output);
    assert_eq!(
        json_line(&output)["source_hashes"]["0"]["sha256"],
        sha(b"/ed-1.22.5.tar.lz")
    );
    assert_file(&input, &original);
    assert!(!root.join("ed.spec").exists());
    success(&run(&[]));
    assert!(
        fs::read_to_string(root.join("ed.spec"))
            .unwrap()
            .contains(&sha(b"/ed-1.22.5.tar.lz"))
    );
    fs::remove_file(root.join("ed.spec")).unwrap();
    manifest["sources"]["0"]
        .as_table_mut()
        .unwrap()
        .insert("sha256".into(), "a".repeat(64).into());
    fs::write(&input, toml::to_string(&manifest).unwrap()).unwrap();
    server.calls.lock().unwrap().clear();
    success(&run(&["--check", "--format", "json"]));
    assert!(server.calls.lock().unwrap().is_empty()); // Declared hashes are never overwritten or verified by gen.
    fs::write(&input, &original).unwrap();
    *mode.lock().unwrap() = "fail";
    let failed = run(&["--check", "--format", "json"]);
    success(&failed);
    let report = json_line(&failed);
    assert!(
        report["authoring_warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("404"))
    );
    assert!(report["source_hashes"]["0"].is_null());
    assert_file(&input, &original);
    for mode_name in ["drift", "drift-fail"] {
        fs::write(&input, &original).unwrap();
        *mode.lock().unwrap() = mode_name;
        let failed = run(&["--check", "--format", "json"]);
        assert_eq!(failed.status.code(), Some(1));
        assert!(
            json_line(&failed)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("changed")
        );
        assert_file(&input, "# concurrent change\n");
        assert!(!root.join("ed.spec").exists());
    }
    fs::write(
        &input,
        original.replace("name = \"ed\"", "name = \"wrong\""),
    )
    .unwrap();
    server.calls.lock().unwrap().clear();
    assert_eq!(run(&["--check"]).status.code(), Some(1));
    assert!(server.calls.lock().unwrap().is_empty()); // Invalid input is rejected before I/O.
}

#[test]
fn edit_hashes_the_pending_candidate_and_keeps_drafts_and_stale_guards() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_owned();
    let input = root.join("input.spec");
    let changed = input.clone();
    let mode = Arc::new(Mutex::new("ok"));
    let current = mode.clone();
    let server = Server::new(false, move |path| {
        let mode = *current.lock().unwrap();
        if mode == "drift" {
            fs::write(&changed, "# concurrent change\n").unwrap();
        }
        if mode == "fail" {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
        } else {
            response(path.as_bytes())
        }
    });
    let source = include_str!("../fixtures/ed.spec")
        .replace("#!RemoteAsset:  sha256:56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc", "#!RemoteAsset")
        .replace("https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz", &format!("{}/%{{version}}.tar", server.url));
    fs::write(&input, &source).unwrap();
    let run = |args: &[&str]| {
        server
            .command()
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap()
    };
    let args = [
        "edit",
        "input.spec",
        "--set",
        "package.version=2",
        "--hash-source",
        "0",
    ];
    let output = run(&[&args[..], &["--check", "--format", "json"]].concat());
    success(&output);
    assert_eq!(
        json_line(&output)["files"][0]["source_hashes"]["sources"]["0"]["sha256"],
        sha(b"/2.tar")
    );
    success(&run(&[&args[..], &["--diff"]].concat()));
    assert_file(&input, &source);
    *mode.lock().unwrap() = "fail";
    assert_eq!(run(&args).status.code(), Some(1));
    assert_file(&input, &source);
    *mode.lock().unwrap() = "ok";
    success(&run(&args));
    let expected = source
        .replace("Version:        1.22.5", "Version:        2")
        .replace(
            "#!RemoteAsset\n",
            &format!("#!RemoteAsset:  sha256:{}\n", sha(b"/2.tar")),
        );
    assert_file(&input, &expected);
    server.calls.lock().unwrap().clear();
    let stale = run(&[
        "edit",
        "input.spec",
        "--hash-source",
        "0",
        "--expect-sha256",
        &sha(source.as_bytes()),
        "--check",
    ]);
    assert_eq!(stale.status.code(), Some(1));
    assert!(server.calls.lock().unwrap().is_empty());
    fs::write(
        &input,
        source.replace("#!RemoteAsset", "#!RemoteAsset:  sha256:INVALID"),
    )
    .unwrap();
    success(&run(&["edit", "input.spec", "--hash-source", "0"])); // Damaged old hashes remain repairable.
    fs::write(&input, &source).unwrap();
    success(&run(&[
        "edit",
        "input.spec",
        "--hash-source",
        "0",
        "--prepare",
        "drafts",
    ]));
    success(&run(&[
        "edit",
        "--from",
        "drafts",
        "--hash-source",
        "0",
        "--check",
    ]));
    assert_file(&input, &source);
    *mode.lock().unwrap() = "drift";
    assert_eq!(run(&args).status.code(), Some(1));
    assert_file(&input, "# concurrent change\n");
    *mode.lock().unwrap() = "ok";
    let batch = source.replace(
        "%description",
        "#!RemoteAsset\nSource1: local.tar\n%description",
    );
    fs::write(&input, &batch).unwrap();
    server.calls.lock().unwrap().clear();
    assert_eq!(
        run(&[
            "edit",
            "input.spec",
            "--hash-source",
            "0",
            "--hash-source",
            "1",
            "--check"
        ])
        .status
        .code(),
        Some(1)
    );
    assert!(server.calls.lock().unwrap().is_empty());
    assert_file(&input, &batch);
}
