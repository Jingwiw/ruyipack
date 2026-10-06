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
        fs::read_to_string(work.join(".cache/ed.candidate.spec"))
            .unwrap()
            .contains(&sha(b"/ed-1.22.5.tar.lz"))
    );
    fs::remove_file(work.join(".cache/ed.candidate.spec")).unwrap();
    manifest["sources"]["0"]
        .as_table_mut()
        .unwrap()
        .insert("sha256".into(), "a".repeat(64).into());
    fs::write(&input, toml::to_string(&manifest).unwrap()).unwrap();
    server.calls.lock().unwrap().clear();
    success(&run(&["--check", "--format", "toml"]));
    assert!(server.calls.lock().unwrap().is_empty()); // Declared hashes are never overwritten or verified by gen.
    let duplicate = manifest["sources"]["0"].clone();
    manifest["sources"]
        .as_table_mut()
        .unwrap()
        .insert("1".into(), duplicate);
    fs::write(&input, toml::to_string(&manifest).unwrap()).unwrap();
    let refreshed = run(&["--hash", "--check", "--format", "toml"]);
    success(&refreshed);
    assert_eq!(
        machine_report(&refreshed)["source_hashes"][0]["sha256"].as_str(),
        Some((sha(b"/ed-1.22.5.tar.lz")).as_str())
    );
    assert_eq!(
        machine_report(&refreshed)["source_hashes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    manifest["sources"].as_table_mut().unwrap().remove("1");
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
    fs::remove_dir_all(root.join(".ruyipack-draft")).unwrap();
    fs::write(
        &input,
        source.replace("#!RemoteAsset", "#!RemoteAsset:  sha256:INVALID"),
    )
    .unwrap();
    success(&run(&["edit", "--spec=input.spec", "--hash-source", "0"])); // Damaged old hashes remain repairable.
    let stage = root.join(".ruyipack-draft/input");
    assert!(stage.join("input.toml").is_file());
    fs::remove_dir_all(root.join(".ruyipack-draft")).unwrap();
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
    fs::remove_dir_all(root.join(".ruyipack-draft")).unwrap();
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
fn edit_hash_all_repairs_old_urls_and_preflights_the_complete_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::new(false, |path| response(path.as_bytes()));
    let replacement = format!("{}/replacement", server.url).replacen("http:", "HTTP:", 1);
    let fixture = include_str!("../fixtures/ed.spec");
    let run = |input: &std::path::Path| {
        server
            .command()
            .args([
                "edit",
                "--spec",
                input.to_str().unwrap(),
                "--set",
                &format!("sources.0.url={replacement}"),
                "--hash",
                "--check",
                "--format",
                "toml",
            ])
            .output()
            .unwrap()
    };
    for (index, old) in [
        "https://fixture-user:fixture-secret@example.invalid/archive",
        "https:/example.invalid/archive",
        "%{unknown_source}",
    ]
    .into_iter()
    .enumerate()
    {
        let input = dir.path().join(format!("repair-{index}.spec"));
        let source = fixture
            .replace("https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz", old)
            .replace("%description", "Source1: local.tar\n%description");
        fs::write(&input, &source).unwrap();
        server.calls.lock().unwrap().clear();
        let output = run(&input);
        success(&output);
        assert_eq!(
            machine_report(&output)["files"][0]["source_hashes"]["sources"][0]["sha256"].as_str(),
            Some(sha(b"/replacement").as_str())
        );
        assert_eq!(*server.calls.lock().unwrap(), ["/replacement"]);
        assert_file(&input, &source);
    }
    for (index, bad) in [
        "#!RemoteAsset\nSource1: https://fixture-user:fixture-secret@example.invalid/archive\n",
        "#!RemoteAsset\nSource1: %{unknown_source}\n",
        "%include unresolved.spec\n",
    ]
    .into_iter()
    .enumerate()
    {
        let input = dir.path().join(format!("invalid-{index}.spec"));
        let source = fixture.replace("%description", &format!("{bad}%description"));
        fs::write(&input, &source).unwrap();
        server.calls.lock().unwrap().clear();
        let output = run(&input);
        assert_eq!(output.status.code(), Some(1));
        assert!(server.calls.lock().unwrap().is_empty());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-secret"));
        assert_file(&input, &source);
    }
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
            "work/gen-work",
        ])
        .output()
        .unwrap();
    success(&prepare);
    let stage = work.join("ed.toml");
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
    let completed = work.join(".cache/ed.resolved.toml");
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
        fs::read_to_string(work.join(".cache/ed.candidate.spec"))
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
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &source);
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
    assert!(
        machine_report(&refreshed)["authoring_warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        machine_report(&refreshed)["source_hashes"][0]["sha256"].as_str(),
        Some(observed.as_str())
    );
    assert_eq!(server.calls.lock().unwrap().len(), 1);
}

#[test]
fn explicit_generation_hashes_preflight_all_sources_before_downloading() {
    let server = Server::new(false, |path| response(path.as_bytes()));
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = include_str!("../fixtures/ed.spec")
        .replace(
            "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
            &format!("{}/archive", server.url),
        )
        .replace(
            "BuildSystem:",
            "#!RemoteAsset\nSource1: https://fixture:secret@example.invalid/archive\nBuildSystem:",
        );
    let work = recipe_workspace(root, "ed", "ed", &source);
    success(&super::support::run(
        root,
        &[
            "edit",
            "ed",
            "--field",
            "package.version",
            "--prepare",
            "work/ed",
        ],
    ));
    let stage = work.join("ed.toml");
    let before = fs::read(&stage).unwrap();
    let output = server
        .command()
        .current_dir(root)
        .args(["gen", "ed", "--hash", "--check", "--format=toml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report = machine_report(&output);
    assert_eq!(report["scope"].as_str(), Some("selected-generation-static"));
    assert_eq!(
        report["source_hash_failures"][0]["source_number"].as_integer(),
        Some(1)
    );
    assert_eq!(
        report["source_hash_failures"][0]["reason"].as_str(),
        Some("url-policy")
    );
    assert!(server.calls.lock().unwrap().is_empty());
    assert_eq!(fs::read(&stage).unwrap(), before);
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &source);
    assert!(!work.join(".cache/ed.resolved.toml").exists());
    fs::write(&stage, "[").unwrap();
    let malformed = super::support::run(root, &["gen", "ed", "--check", "--format=toml"]);
    assert_eq!(malformed.status.code(), Some(1));
    assert_eq!(
        machine_report(&malformed)["scope"].as_str(),
        Some("selected-generation-static")
    );
    assert_file(&stage, "[");
}

#[test]
fn edit_partial_hash_failure_reports_observations_without_publishing_digests() {
    let directory = tempfile::tempdir().unwrap();
    let server = Server::new(true, |path| {
        if path == "/fail" {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
        } else {
            response(path.as_bytes())
        }
    });
    let source = include_str!("../fixtures/ed.spec")
        .replace(
            "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
            &format!("{}/ok", server.url),
        )
        .replace(
            "BuildSystem:",
            &format!(
                "#!RemoteAsset:  sha256:{}\nSource1: {}/fail\nBuildSystem:",
                "a".repeat(64),
                server.url
            ),
        );
    let work = recipe_workspace(directory.path(), "ed", "ed", &source);
    let run = |extra: &[&str]| {
        server
            .command()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .current_dir(directory.path())
            .arg("edit")
            .args(extra)
            .output()
            .unwrap()
    };
    success(&run(&["ed", "--prepare", "drafts"]));
    let draft = directory.path().join("drafts/ed.toml");
    let before = fs::read_to_string(&draft).unwrap();
    let output = run(&["--from", "drafts", "--hash", "--apply", "--format=toml"]);
    assert_eq!(output.status.code(), Some(1));
    let report = machine_report(&output);
    let file = &report["files"][0];
    let observed = sha(b"/ok");
    assert_eq!(
        file["source_hashes"]["sources"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        file["source_hashes"]["sources"][0]["sha256"].as_str(),
        Some(observed.as_str())
    );
    assert_eq!(file["error"]["source_number"].as_integer(), Some(1));
    assert_eq!(file["error"]["http_status"].as_integer(), Some(404));
    assert_file(&draft, &before);
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &source);
    assert_eq!(*server.calls.lock().unwrap(), ["/ok", "/fail"]);
}

#[test]
fn check_repairs_only_missing_digests_and_retries_without_downloads() {
    let server = hash_server();
    let dir = tempfile::tempdir().unwrap();
    let fixture = include_str!("../fixtures/ed.spec");
    let source = fixture
        .replace("https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz", &format!("{}/asset", server.url))
        .replace("#!RemoteAsset:  sha256:56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc", "#!RemoteAsset");
    assert!(source.contains("#!RemoteAsset\n"));
    let work = recipe_workspace(dir.path(), "repair", "ed", &source);
    let run = || {
        server
            .command()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .current_dir(dir.path())
            .args(["check", "repair", "--auto-fix", "--format", "toml"])
            .output()
            .unwrap()
    };
    let first = run();
    success(&first);
    let report = machine_report(&first);
    assert_eq!(report["operation"].as_str(), Some("auto-fix"));
    assert_eq!(report["files"][0]["created_work"].as_bool(), Some(true));
    let fixed = source.replace(
        "#!RemoteAsset\n",
        &format!("#!RemoteAsset:  sha256:{}\n", sha(b"\0\xffasset\n")),
    );
    let spec = work.join("recipe/SPECS/ed/ed.spec");
    assert_file(&spec, &fixed);
    let calls = server.calls.lock().unwrap().len();
    success(&run());
    assert_file(&spec, &fixed);
    assert_eq!(server.calls.lock().unwrap().len(), calls);
    assert_file(dir.path().join("openruyi/SPECS/ed/ed.spec"), &source);
}

#[test]
fn check_repair_keeps_recipe_on_download_failure_and_refuses_pending_edits() {
    let server = hash_server();
    let dir = tempfile::tempdir().unwrap();
    let source = include_str!("../fixtures/ed.spec").replace(
        "%description\n",
        &format!(
            "#!RemoteAsset\nSource1: {}/fail\n\n%description\n",
            server.url
        ),
    );
    let work = recipe_workspace(dir.path(), "repair", "ed", &source);
    let run = || {
        server
            .command()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .current_dir(dir.path())
            .args(["check", "repair", "--auto-fix", "--format", "toml"])
            .output()
            .unwrap()
    };
    let failed = run();
    assert_eq!(failed.status.code(), Some(1));
    assert_eq!(machine_report(&failed)["success"].as_bool(), Some(false));
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &source);
    assert_eq!(*server.calls.lock().unwrap(), ["/fail"]);
    let input = work.join("ed.toml");
    let mut values: toml::Table = toml::from_str(&fs::read_to_string(&input).unwrap()).unwrap();
    values["sources"]["1"]["sha256"] = sha(b"pending").into();
    let pending = toml::to_string(&values).unwrap();
    fs::write(&input, &pending).unwrap();
    let refused = run();
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("pending TOML edits"));
    assert_file(input, &pending);
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &source);
    assert_eq!(*server.calls.lock().unwrap(), ["/fail"]);
}

#[test]
fn check_repair_does_not_apply_unrelated_authoring_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let work = authoring_workspace(
        dir.path(),
        "authoring",
        "ed",
        include_str!("../../examples/ed/ed.toml"),
    );
    success(
        &command()
            .current_dir(dir.path())
            .args(["gen", "authoring", "--offline", "--apply"])
            .output()
            .unwrap(),
    );
    let spec = work.join("recipe/SPECS/ed/ed.spec");
    let original = fs::read(&spec).unwrap();
    let input = work.join("ed.toml");
    let pending = format!(
        "{}\n[build.stages.build]\nappend = 'echo pending-script'\n",
        fs::read_to_string(&input).unwrap()
    );
    fs::write(&input, &pending).unwrap();
    let refused = command()
        .current_dir(dir.path())
        .args(["check", "authoring", "--auto-fix", "--format", "toml"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("pending TOML edits"));
    assert_file(input, &pending);
    assert_eq!(fs::read(spec).unwrap(), original);
}

#[test]
fn auto_fix_summary_is_literal_local_and_repeatable() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = include_str!("../fixtures/ed.spec");
    let summary = fixture
        .lines()
        .find(|line| line.starts_with("Summary:"))
        .unwrap();
    let source = fixture.replace(summary, &format!("{summary}."));
    let work = recipe_workspace(directory.path(), "repair", "ed", &source);
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_ruyipack"))
            .current_dir(directory.path())
            .args(["check", "repair", "--auto-fix", "--format", "toml"])
            .output()
            .unwrap()
    };
    let first = run();
    success(&first);
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), fixture);
    let report = machine_report(&first);
    assert!(
        report["files"][0]["changed_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field.as_str() == Some("package.summary"))
    );
    success(&run());
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), fixture);
}

#[test]
fn version_warning_preserves_static_policy_and_recipe() {
    let directory = tempfile::tempdir().unwrap();
    let source = include_str!("../fixtures/ed.spec")
        .replace("Name:           ed", "Name:           %{unknown_pkg}")
        .replace(
            "Summary:        A line-oriented text editor",
            "Summary:        A line-oriented text editor.",
        );
    let work = recipe_workspace(directory.path(), "review", "ed", &source);
    for (policy, exit) in [("authoring", 0), ("submit", 1)] {
        let output = command()
            .current_dir(directory.path())
            .args([
                "check",
                "review",
                "--upgrade",
                "--policy",
                policy,
                "--format",
                "toml",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit));
        let report = machine_report(&output);
        assert_eq!(report["upgrade"]["status"].as_str(), Some("unavailable"));
        assert!(
            report["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["code"].as_str() == Some("RPK006"))
        );
        assert_file(directory.path().join("openruyi/SPECS/ed/ed.spec"), &source);
        assert!(!work.join("recipe").exists());
    }
}

#[test]
fn basic_fix_does_not_resolve_downloads_when_digests_exist() {
    let directory = tempfile::tempdir().unwrap();
    let source = include_str!("../fixtures/ed.spec")
        .replace(
            "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
            "%{unknown_source}",
        )
        .replace(
            "Summary:        A line-oriented text editor",
            "Summary:        A line-oriented text editor.",
        );
    let work = recipe_workspace(directory.path(), "repair", "ed", &source);
    let output = command()
        .current_dir(directory.path())
        .args(["check", "repair", "--auto-fix", "--format", "toml"])
        .output()
        .unwrap();
    success(&output);
    assert_file(
        work.join("recipe/SPECS/ed/ed.spec"),
        &source.replace(
            "Summary:        A line-oriented text editor.",
            "Summary:        A line-oriented text editor",
        ),
    );
}

#[test]
fn basic_fix_removes_unused_signature_without_fetching_and_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let source = include_str!("../fixtures/ed.spec").replace(
        "BuildSystem:    autotools",
        "#!RemoteAsset\nSource1: https://invalid.example/ed.sig.asc\nBuildSystem:    autotools",
    );
    let work = recipe_workspace(directory.path(), "signature", "ed", &source);
    let original = directory.path().join("openruyi/SPECS/ed/ed.sig.asc");
    std::fs::write(&original, "local signature").unwrap();
    crate::support::git(&directory.path().join("openruyi"), &["add", "."]);
    crate::support::git(
        &directory.path().join("openruyi"),
        &["commit", "-m", "Add signature material"],
    );
    for _ in 0..2 {
        let output = command()
            .current_dir(directory.path())
            .args(["check", "signature", "--auto-fix", "--format", "toml"])
            .output()
            .unwrap();
        success(&output);
        assert_file(
            work.join("recipe/SPECS/ed/ed.spec"),
            &source.replace(
                "#!RemoteAsset\nSource1: https://invalid.example/ed.sig.asc\n",
                "",
            ),
        );
        assert!(!work.join("recipe/SPECS/ed/ed.sig.asc").exists());
    }
    assert_file(&original, "local signature");
}
