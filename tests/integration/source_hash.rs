// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Actual loopback downloads exercise completion, failure and stale-input guards.

use super::{
    http::{Server, response},
    support::{
        assert_file, authoring_workspace, command, machine_report, recipe_workspace, success,
    },
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
fn invalid_ca_bundles_identify_the_file_without_changing_input() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.spec");
    let source = "Name: probe\nSource0: https://example.invalid/archive\n";
    fs::write(&input, source).unwrap();
    let ca = dir.path().join("custom CA.pem");
    for (contents, diagnostic) in [
        (None, "open file"),
        (Some(""), "no certificates"),
        (
            Some("-----BEGIN CERTIFICATE-----\n!\n-----END CERTIFICATE-----"),
            "pem",
        ),
    ] {
        if let Some(contents) = contents {
            fs::write(&ca, contents).unwrap();
        }
        let output = command()
            .env("SSL_CERT_FILE", &ca)
            .args(["source", "hash", "--spec"])
            .arg(&input)
            .args(["--format", "toml"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let report = machine_report(&output);
        let error = &report["error"];
        let message = error["message"].as_str().unwrap();
        assert!(
            message.contains(ca.to_str().unwrap()) && message.contains("SSL_CERT_FILE"),
            "{message}"
        );
        assert!(message.to_lowercase().contains(diagnostic), "{message}");
        assert_eq!(error["reason"].as_str(), Some("tls"));
        assert_eq!(error["retryable"].as_bool(), Some(false));
        assert!(report.get("sha256").is_none());
        assert_file(&input, source);
    }
}

fn hash_server() -> Server {
    let attempts = Cell::new(0);
    let redirects = Cell::new(0);
    Server::new(true, move |path| {
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
    })
}

#[test]
fn streaming_hashes_validate_tls_redirects_and_complete_response_bodies() {
    let server = hash_server();
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.spec");
    let source = format!(
        "Name: probe\nVersion: 1\n%global route redirect\nSource3: local.tar\nSource: {}/%{{route}}#/renamed.tar\n%description\nProbe\n",
        server.url
    );
    let run = || {
        server
            .command()
            .args(["source", "hash", "--spec"])
            .arg(&input)
            .args(["--source", "4", "--format", "toml"])
            .output()
            .unwrap()
    };
    fs::write(&input, &source).unwrap();
    let output = run();
    success(&output);
    let report = machine_report(&output);
    assert_eq!(
        report["sha256"].as_str(),
        Some((sha(b"\0\xffasset\n")).as_str())
    );
    assert_eq!(report["bytes"].as_integer(), Some(8));
    assert_eq!(
        report["effective_url"].as_str(),
        Some((format!("{}/asset", server.url)).as_str())
    );
    assert_eq!(*server.calls.lock().unwrap(), ["/redirect", "/asset"]);
    assert_file(&input, &source);
    for (path, message, reason, retryable, status) in [
        ("fail", "404", "http-status", false, Some(404)),
        ("busy", "503", "http-status", true, Some(503)),
        ("truncated", "download body", "body-read", true, None),
        ("partial", "206", "http-status", false, Some(206)),
        ("loop", "redirects", "redirect-limit", false, None),
        ("bad-redirect", "credentials", "url-policy", false, None),
        ("downgrade", "downgrade", "url-policy", false, None),
    ] {
        fs::write(
            &input,
            source.replace("route redirect", &format!("route {path}")),
        )
        .unwrap();
        let output = run();
        assert_eq!(output.status.code(), Some(1), "{path}: {output:?}");
        let report = machine_report(&output);
        assert!(report.get("sha256").is_none());
        assert!(
            report["error"]["message"]
                .as_str()
                .unwrap()
                .contains(message),
            "{report}"
        );
        assert_eq!(report["error"]["reason"].as_str(), Some(reason));
        assert_eq!(report["error"]["retryable"].as_bool(), Some(retryable));
        assert_eq!(
            report["error"]
                .get("http_status")
                .and_then(toml::Value::as_integer),
            status.map(i64::from)
        );
        assert_eq!(report["error"]["source_number"].as_integer(), Some(4));
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
    for (route, bytes) in [
        ("retry", b"\0\xffasset\n".as_slice()),
        ("encoding", b"raw!"),
    ] {
        fs::write(
            &input,
            source.replace("route redirect", &format!("route {route}")),
        )
        .unwrap();
        let output = run();
        success(&output);
        assert_eq!(
            machine_report(&output)["sha256"].as_str(),
            Some(sha(bytes).as_str())
        );
    }
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
}

#[test]
fn untrusted_tls_is_rejected_as_non_retryable() {
    let server = hash_server();
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.spec");
    fs::write(&input, format!("Source4: {}/encoding\n", server.url)).unwrap();
    let rejected = server
        .command()
        .env_remove("SSL_CERT_FILE")
        .args(["source", "hash", "--spec"])
        .arg(&input)
        .args(["--source", "4", "--format", "toml"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(1)); // Never silently accept an untrusted TLS peer.
    let error = &machine_report(&rejected)["error"];
    assert_eq!(error["stage"].as_str(), Some("download"));
    assert_eq!(error["reason"].as_str(), Some("tls"));
    assert_eq!(error["retryable"].as_bool(), Some(false));
}

#[test]
fn generation_completes_only_missing_hashes_and_never_publishes_stale_input() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_owned();
    let work = authoring_workspace(
        &root,
        "authoring",
        "ed",
        include_str!("../../examples/ed/ed.toml"),
    );
    let input = work.join("ed.toml");
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
            .args(["gen", "authoring"])
            .args(extra)
            .output()
            .unwrap()
    };
    success(&run(&["--offline", "--stdout"]));
    assert!(server.calls.lock().unwrap().is_empty());
    let output = run(&["--check", "--format", "toml"]);
    success(&output);
    assert_eq!(
        machine_report(&output)["source_hashes"][0]["sha256"].as_str(),
        Some((sha(b"/ed-1.22.5.tar.lz")).as_str())
    );
    assert_file(&input, &original);
    assert!(!root.join("ed.spec").exists());
    success(&run(&[]));
    assert!(
        fs::read_to_string(work.join("stage/ed.candidate.spec"))
            .unwrap()
            .contains(&sha(b"/ed-1.22.5.tar.lz"))
    );
    fs::remove_file(work.join("stage/ed.candidate.spec")).unwrap();
    manifest["sources"]["0"]
        .as_table_mut()
        .unwrap()
        .insert("sha256".into(), "a".repeat(64).into());
    fs::write(&input, toml::to_string(&manifest).unwrap()).unwrap();
    server.calls.lock().unwrap().clear();
    success(&run(&["--check", "--format", "toml"]));
    assert!(server.calls.lock().unwrap().is_empty()); // Declared hashes are never overwritten or verified by gen.
    let refreshed = run(&["--hash", "--check", "--format", "toml"]);
    success(&refreshed);
    assert_eq!(
        machine_report(&refreshed)["source_hashes"][0]["sha256"].as_str(),
        Some((sha(b"/ed-1.22.5.tar.lz")).as_str())
    );
    assert_eq!(server.calls.lock().unwrap().len(), 1);
    let offline_hash = run(&["--offline", "--hash"]);
    assert_eq!(offline_hash.status.code(), Some(2));
    fs::write(&input, &original).unwrap();
    *mode.lock().unwrap() = "fail";
    let failed = run(&["--check", "--format", "toml"]);
    success(&failed);
    let report = machine_report(&failed);
    assert_eq!(
        report["source_hash_failures"][0]["reason"].as_str(),
        Some("http-status")
    );
    assert_eq!(
        report["source_hash_failures"][0]["http_status"].as_integer(),
        Some(404)
    );
    assert_eq!(report["valid"].as_bool(), Some(true)); // A failed best-effort hash remains a warning.
    assert!(report["source_hashes"].as_array().unwrap().is_empty());
    assert_file(&input, &original);
    for mode_name in ["drift", "drift-fail"] {
        fs::write(&input, &original).unwrap();
        *mode.lock().unwrap() = mode_name;
        let failed = run(&["--check", "--format", "toml"]);
        assert_eq!(failed.status.code(), Some(1));
        assert!(
            machine_report(&failed)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("changed")
        );
        assert_file(&input, "# concurrent change\n");
        assert!(!root.join("ed.spec").exists());
    }
    fs::write(
        &input,
        original.replace("version = \"1.22.5\"", "version = \"\""),
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
        "--spec=input.spec",
        "--set",
        "package.version=2",
        "--hash-source",
        "0",
        "--hash-source",
        "0",
    ];
    let output = run(&[&args[..], &["--check", "--format", "toml"]].concat());
    success(&output);
    assert_eq!(
        machine_report(&output)["files"][0]["source_hashes"]["sources"][0]["sha256"].as_str(),
        Some((sha(b"/2.tar")).as_str())
    );
    assert_eq!(*server.calls.lock().unwrap(), ["/2.tar"]);
    success(&run(&[&args[..], &["--diff"]].concat()));
    assert_file(&input, &source);
    *mode.lock().unwrap() = "fail";
    let output = run(&[&args[..], &["--check", "--format", "toml"]].concat());
    assert_eq!(output.status.code(), Some(1));
    let error = &machine_report(&output)["files"][0]["error"];
    assert_eq!(error["code"].as_str(), Some("source-hash-failed"));
    assert_eq!(error["stage"].as_str(), Some("download"));
    assert_eq!(error["reason"].as_str(), Some("http-status"));
    assert_eq!(error["http_status"].as_integer(), Some(404));
    assert_eq!(error["retryable"].as_bool(), Some(false));
    assert_file(&input, &source);
    *mode.lock().unwrap() = "ok";
    success(&run(&[&args[..], &["--apply"]].concat()));
    let expected = source
        .replace("Version:        1.22.5", "Version:        2")
        .replace(
            "#!RemoteAsset\n",
            &format!("#!RemoteAsset:  sha256:{}\n", sha(b"/2.tar")),
        );
    assert_file(&input, &expected);
    server.calls.lock().unwrap().clear();
    let stale_result = run(&[
        "edit",
        "--spec=input.spec",
        "--hash-source",
        "0",
        "--expect-sha256",
        &sha(source.as_bytes()),
        "--check",
    ]);
    assert_eq!(stale_result.status.code(), Some(1));
    assert!(server.calls.lock().unwrap().is_empty());
    fs::remove_dir_all(root.join(".ruyipack-stage")).unwrap();
    fs::write(
        &input,
        source.replace("#!RemoteAsset", "#!RemoteAsset:  sha256:INVALID"),
    )
    .unwrap();
    success(&run(&["edit", "--spec=input.spec", "--hash-source", "0"])); // Damaged old hashes remain repairable.
    let stage = root.join(".ruyipack-stage/input");
    assert!(stage.join("input.toml").is_file());
    fs::remove_dir_all(root.join(".ruyipack-stage")).unwrap();
    fs::write(&input, &source).unwrap();
    success(&run(&[
        "edit",
        "--spec=input.spec",
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
    fs::remove_dir_all(root.join(".ruyipack-stage")).unwrap();
    fs::write(&input, &batch).unwrap();
    server.calls.lock().unwrap().clear();
    assert_eq!(
        run(&[
            "edit",
            "--spec=input.spec",
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

#[test]
fn generation_hash_completion_uses_edited_urls_and_never_backfills_stale_baseline_digest() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let server = Server::new(true, |path| response(path.as_bytes()));
    let source = include_str!("../fixtures/ed.spec").replace(
        "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
        &format!("{}/ed-%{{version}}.tar", server.url),
    );
    let work = recipe_workspace(root, "gen-work", "ed", &source);
    let prepare = server
        .command()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .current_dir(root)
        .args([
            "edit",
            "gen-work",
            "--field",
            "package.version",
            "--field",
            "sources.0",
            "--prepare",
            "work/gen-work/stage",
        ])
        .output()
        .unwrap();
    success(&prepare);
    let stage = work.join("stage/ed.toml");
    let edited = format!(
        "[package]\nversion = '2'\n[sources.0]\nurl = '{}/ed-%{{version}}.tar'\n",
        server.url
    );
    fs::write(&stage, &edited).unwrap();
    let run = |extra: &[&str]| {
        server
            .command()
            .current_dir(root)
            .args(["gen", "gen-work"])
            .args(extra)
            .output()
            .unwrap()
    };
    success(&run(&["--offline"]));
    assert!(server.calls.lock().unwrap().is_empty());
    let completed = work.join("stage/ed.resolved.toml");
    let document: toml::Value = toml::from_str(&fs::read_to_string(&completed).unwrap()).unwrap();
    assert!(
        document["edit"]["values"]["sources"]["0"]
            .as_table()
            .unwrap()
            .get("sha256")
            .is_none()
    );
    let baseline_digest = "56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc";
    assert!(
        fs::read_to_string(work.join("stage/ed.candidate.spec"))
            .unwrap()
            .contains(baseline_digest)
    );
    success(&run(&[]));
    let observed = sha(b"/ed-2.tar");
    let document: toml::Value = toml::from_str(&fs::read_to_string(&completed).unwrap()).unwrap();
    assert_eq!(
        document["edit"]["values"]["sources"]["0"]["sha256"].as_str(),
        Some(observed.as_str())
    );
    assert_eq!(document["role"].as_str(), Some("resolved-edit"));
    assert_eq!(document["downloads"][0]["number"].as_integer(), Some(0));
    assert_eq!(
        document["downloads"][0]["sha256"].as_str(),
        Some(observed.as_str())
    );
    assert_file(&stage, &edited);
    assert_file(work.join("checkout/SPECS/ed/ed.spec"), &source);
    server.calls.lock().unwrap().clear();
    // An explicit digest remains a declaration; --hash alone requests re-observation.
    fs::write(&stage, format!("{edited}sha256 = '{baseline_digest}'\n")).unwrap();
    let checked = run(&["--check", "--format=toml"]);
    success(&checked);
    assert!(server.calls.lock().unwrap().is_empty());
    assert!(
        machine_report(&checked)["authoring_warnings"][0]
            .as_str()
            .unwrap()
            .contains("declaration, not verification")
    );
    let refreshed = run(&["--hash", "--check", "--format=toml"]);
    success(&refreshed);
    assert_eq!(
        machine_report(&refreshed)["source_hashes"][0]["sha256"].as_str(),
        Some(observed.as_str())
    );
    assert_eq!(server.calls.lock().unwrap().len(), 1);
}
