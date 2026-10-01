// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared Source/RemoteAsset behavior across real generation and editing commands.

use super::support::{assert_file, authoring_workspace, quiet_success, rejected, run, success};

use std::{fs, process::Output};

const MANIFEST: &str = include_str!("../../examples/ed/ed.toml");
const SPEC: &str = include_str!("../fixtures/ed.spec");
const URL: &str = "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz";
const HASH: &str = "56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc";

fn reviewed(output: &Output, field: &str) {
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("review required after changing") && stderr.contains(field),
        "{stderr}"
    );
}

#[test]
fn generated_sources_are_viewable_and_round_trip_without_changing_a_byte() {
    for url in [
        "https://example.org/%{?name}",
        "%{url}/download#/%{name}-%{version}.tar.lz",
        "%url/download#/%name-%version.tar.lz",
        "https://example.org/%{name}/a%%20b-%{version}.tar.lz#/%{name}.tar.lz",
        "https://example.org/a%%2Fb%%3F%%23%%25%%41.tar.lz",
        "https://example.org/archive.tar.lz#/%{name}-%{version}.tar.lz",
    ] {
        let directory = tempfile::tempdir().unwrap();
        authoring_workspace(
            directory.path(),
            "review",
            "ed",
            &MANIFEST.replace(URL, url),
        );
        let generated = run(
            directory.path(),
            &["gen", "review", "--offline", "--stdout"],
        );
        quiet_success(&generated);
        assert_eq!(generated.stdout, SPEC.replace(URL, url).as_bytes());
        fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
        let view = run(
            directory.path(),
            &["inspect", "--spec=ed.spec", "--editable", "--all"],
        );
        quiet_success(&view);
        let document: toml::Table =
            toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
        assert_eq!(document["sources"]["0"]["url"].as_str(), Some(url));
        let unchanged = run(
            directory.path(),
            &[
                "edit",
                "--spec=ed.spec",
                "--set",
                &format!("sources.0.url={url}"),
                "--stdout",
            ],
        );
        success(&unchanged);
        assert_eq!(unchanged.stdout, generated.stdout);
        assert_eq!(
            fs::read(directory.path().join("ed.spec")).unwrap(),
            generated.stdout
        );
    }
}

#[test]
fn invalid_or_unsupported_sources_can_be_viewed_but_not_published() {
    for url in [
        "https://",
        "https://bad host/file",
        "https://%20/file",
        "https:/example.org/file",
        "https:example.org/file",
        "https:///example.org/file",
        "https://example.org/a b",
        "https://example.org/\\file",
        "https://example.org/%",
        "https://example.org/%{unknown}",
        "https://example.org/file#/%{unknown}",
        "https://example.org/a%20b#/%{unknown}",
        "https://example.org/%{name extra}",
        "https://example.org/%{lua:print(123)}",
        "https://example.org/%(touch MUST_NOT_EXIST)",
        "https://example.org/%[1+1]",
        "https://example.org/%unknown",
        "https://example.org/%AF",
        "https://example.org/%bad",
        "https://example.org/a%20b.tar.lz",
        "https://example.org/%{version",
        "ftp://example.org/file",
        "/tmp/source.tar.lz",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut manifest: toml::Table = toml::from_str(MANIFEST).unwrap();
        manifest["sources"]["0"]["url"] = url.into();
        authoring_workspace(
            directory.path(),
            "review",
            "ed",
            &toml::to_string(&manifest).unwrap(),
        );
        rejected(
            &run(directory.path(), &["gen", "review", "--stdout"]),
            "sources.0.url",
        );
        let spec = SPEC.replace(URL, url);
        let _ = fs::remove_dir_all(directory.path().join(".ruyipack-stage"));
        fs::write(directory.path().join("ed.spec"), &spec).unwrap();
        let view = run(
            directory.path(),
            &["inspect", "--spec=ed.spec", "--editable", "--all"],
        );
        quiet_success(&view);
        let document: toml::Table =
            toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
        assert_eq!(document["sources"]["0"]["url"].as_str(), Some(url));
        assert_file(directory.path().join("ed.spec"), &spec);
        fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
        rejected(
            &run(
                directory.path(),
                &[
                    "edit",
                    "--spec=ed.spec",
                    "--set",
                    &format!("sources.0.url={url}"),
                    "--apply",
                ],
            ),
            "sources.0.url",
        );
        assert_file(directory.path().join("ed.spec"), SPEC);
        assert!(!directory.path().join("MUST_NOT_EXIST").exists());
    }
}

#[test]
fn selected_source_repairs_validate_the_new_url_not_the_old_one() {
    let directory = tempfile::tempdir().unwrap();
    for (old, replacement) in [
        ("https://", "https://example.org/fixed.tar.lz"),
        (
            "https://example.org/a%20b.tar.lz",
            "https://example.org/a%%20b.tar.lz",
        ),
    ] {
        let _ = fs::remove_dir_all(directory.path().join(".ruyipack-stage"));
        let original = SPEC.replace(URL, old);
        fs::write(directory.path().join("ed.spec"), &original).unwrap();
        let same = format!("sources.0.url={old}");
        rejected(
            &run(
                directory.path(),
                &["edit", "--spec=ed.spec", "--set", &same, "--apply"],
            ),
            "sources.0.url",
        );
        assert_file(directory.path().join("ed.spec"), &original);
        let assignment = format!("sources.0.url={replacement}");
        let preview = run(
            directory.path(),
            &["edit", "--spec=ed.spec", "--set", &assignment, "--stdout"],
        );
        reviewed(&preview, "sources.0.url");
        assert_eq!(preview.stdout, SPEC.replace(URL, replacement).as_bytes());
        assert_file(directory.path().join("ed.spec"), &original);
        let saved = run(
            directory.path(),
            &["edit", "--spec=ed.spec", "--set", &assignment, "--apply"],
        );
        assert!(saved.status.success(), "{saved:?}");
        assert_file(
            directory.path().join("ed.spec"),
            &(SPEC.replace(URL, replacement)),
        );
    }
}

#[test]
fn generated_bare_sources_remain_editable_without_inventing_a_digest() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = MANIFEST.replace(&format!("sha256 = \"{HASH}\"\n"), "");
    authoring_workspace(directory.path(), "review", "ed", &manifest);
    let generated = run(
        directory.path(),
        &["gen", "review", "--offline", "--stdout"],
    );
    assert!(generated.status.success(), "{generated:?}");
    assert!(String::from_utf8_lossy(&generated.stderr).contains("no sha256"));
    assert!(String::from_utf8_lossy(&generated.stderr).contains("openRuyi requires SHA-256"));
    let spec = String::from_utf8(generated.stdout).unwrap();
    assert_eq!(
        spec,
        SPEC.replace(&format!("#!RemoteAsset:  sha256:{HASH}"), "#!RemoteAsset")
    );
    fs::write(directory.path().join("ed.spec"), &spec).unwrap();
    let check = run(
        directory.path(),
        &["check", "--spec=ed.spec", "--format", "toml"],
    );
    assert!(check.status.success());
    let report = super::support::machine_report(&check);
    assert_eq!(report["evidence"]["status"].as_str(), Some("pass"));
    let finding = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"].as_str() == Some("RPK005"))
        .unwrap();
    assert_eq!(finding["severity"].as_str(), Some("warn"));
    let start = spec.find("Source0:").unwrap();
    assert_eq!(
        finding["span"]["start_byte"].as_integer(),
        Some(i64::try_from(start).unwrap())
    );
    assert_eq!(
        finding["span"]["start_line"].as_integer(),
        Some(i64::try_from(spec[..start].bytes().filter(|&b| b == b'\n').count() + 1).unwrap())
    );
    for fields in [vec!["--all"], vec!["--field", "sources.0.url"]] {
        let full = fields == ["--all"];
        let mut args = vec!["inspect", "--spec=ed.spec", "--editable"];
        args.extend(fields);
        let view = run(directory.path(), &args);
        quiet_success(&view);
        let document: toml::Table =
            toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
        let source = document["sources"]["0"].as_table().unwrap();
        assert_eq!(source.len(), if full { 2 } else { 1 });
        if full {
            assert_eq!(source["sha256"].as_str(), Some(""));
        }
        assert_eq!(source["url"].as_str(), Some(URL));
    }
    let assignment = format!("sources.0.sha256={HASH}");
    let preview = run(
        directory.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            &assignment,
            "--stdout",
            "--check",
        ],
    );
    success(&preview);
    assert!(
        String::from_utf8_lossy(&preview.stderr)
            .contains("warnings: new 0, inherited 0, resolved 1"),
        "{preview:?}"
    );
    assert_eq!(preview.stdout, SPEC.as_bytes());
    assert_file(directory.path().join("ed.spec"), &spec);
    rejected(
        &run(
            directory.path(),
            &[
                "edit",
                "--spec=ed.spec",
                "--set",
                "sources.0.sha256=invalid",
                "--apply",
            ],
        ),
        "sha256",
    );
    assert_file(directory.path().join("ed.spec"), &spec);
    assert!(
        run(
            directory.path(),
            &["edit", "--spec=ed.spec", "--set", &assignment, "--apply"]
        )
        .status
        .success()
    );
    assert_file(directory.path().join("ed.spec"), SPEC);
    fs::remove_dir_all(directory.path().join(".ruyipack-stage")).unwrap();
    fs::write(directory.path().join("ed.spec"), &spec).unwrap();
    let replacement = "https://example.org/replacement.tar.lz";
    let assignment = format!("sources.0.url={replacement}");
    let edited = run(
        directory.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            &assignment,
            "--stdout",
            "--check",
        ],
    );
    assert!(edited.status.success(), "{edited:?}");
    assert!(String::from_utf8_lossy(&edited.stderr).contains("RPK005"));
    assert_eq!(edited.stdout, spec.replace(URL, replacement).as_bytes());
    assert_file(directory.path().join("ed.spec"), &spec);
    let saved = run(
        directory.path(),
        &["edit", "--spec=ed.spec", "--set", &assignment, "--apply"],
    );
    assert!(saved.status.success(), "{saved:?}");
    assert_file(
        directory.path().join("ed.spec"),
        &(spec.replace(URL, replacement)),
    );
    for (malformed, error) in [
        (
            spec.replace("#!RemoteAsset\n", "#!RemoteAsset extra\n"),
            "RemoteAsset",
        ),
        (
            spec.replace("#!RemoteAsset\n", "#!RemoteAsset\n#!RemoteAsset\n"),
            "RemoteAsset",
        ),
        (
            spec.replace("#!RemoteAsset\n", "#!RemoteAsset\n\n"),
            "RemoteAsset",
        ),
        (
            spec.replace(
                "BuildSystem:",
                &format!("#!RemoteAsset\nSource0: {URL}\nBuildSystem:"),
            ),
            "duplicate Source identity",
        ),
    ] {
        fs::write(directory.path().join("ed.spec"), &malformed).unwrap();
        rejected(
            &run(
                directory.path(),
                &["inspect", "--spec=ed.spec", "--editable", "--all"],
            ),
            error,
        );
        assert_file(directory.path().join("ed.spec"), &malformed);
    }
}

#[test]
fn existing_http_sources_remain_editable_but_new_generation_requires_https() {
    let url = "http://example.org/%{name}-%{version}.tar.lz";
    let directory = tempfile::tempdir().unwrap();
    authoring_workspace(
        directory.path(),
        "review",
        "ed",
        &MANIFEST.replace(URL, url),
    );
    rejected(
        &run(directory.path(), &["gen", "review", "--stdout"]),
        "expected an HTTPS URL",
    );
    let spec = SPEC.replace(URL, url);
    fs::write(directory.path().join("ed.spec"), &spec).unwrap();
    quiet_success(&run(
        directory.path(),
        &["inspect", "--spec=ed.spec", "--editable", "--all"],
    ));
    let edited = run(
        directory.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            "package.version=1.22.6",
            "--stdout",
        ],
    );
    reviewed(&edited, "package.version");
    assert_eq!(
        edited.stdout,
        spec.replace("Version:        1.22.5", "Version:        1.22.6")
            .as_bytes()
    );
}

#[test]
fn sha256_case_is_preserved_and_non_hex_values_are_rejected() {
    for digest in [HASH.to_uppercase(), "aB".repeat(32), "0".repeat(64)] {
        let directory = tempfile::tempdir().unwrap();
        authoring_workspace(
            directory.path(),
            "review",
            "ed",
            &MANIFEST.replace(HASH, &digest),
        );
        let generated = run(
            directory.path(),
            &["gen", "review", "--offline", "--stdout"],
        );
        quiet_success(&generated);
        assert_eq!(generated.stdout, SPEC.replace(HASH, &digest).as_bytes());
        fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
        let view = run(
            directory.path(),
            &["inspect", "--spec=ed.spec", "--editable", "--all"],
        );
        quiet_success(&view);
        let document: toml::Table =
            toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
        assert_eq!(
            document["sources"]["0"]["sha256"].as_str(),
            Some(digest.as_str())
        );
        let unchanged = run(
            directory.path(),
            &[
                "edit",
                "--spec=ed.spec",
                "--set",
                &format!("sources.0.sha256={digest}"),
                "--stdout",
            ],
        );
        success(&unchanged);
        assert_eq!(unchanged.stdout, generated.stdout);
    }
    for digest in ["g".repeat(64), "a".repeat(63), "a".repeat(65)] {
        let directory = tempfile::tempdir().unwrap();
        authoring_workspace(
            directory.path(),
            "review",
            "ed",
            &MANIFEST.replace(HASH, &digest),
        );
        rejected(
            &run(directory.path(), &["gen", "review", "--stdout"]),
            "64 hexadecimal digits",
        );
        fs::write(
            directory.path().join("ed.spec"),
            SPEC.replace(HASH, &digest),
        )
        .unwrap();
        let output = run(
            directory.path(),
            &["check", "--spec=ed.spec", "--format", "toml"],
        );
        quiet_success(&output);
        let report = super::support::machine_report(&output);
        assert_eq!(report["evidence"]["status"].as_str(), Some("pass"));
        let findings = report["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["code"].as_str(), Some("RPK005"));
        assert_eq!(findings[0]["severity"].as_str(), Some("warn"));
        assert!(
            findings[0]["message"]
                .as_str()
                .unwrap()
                .contains("invalid sha256")
        );
        let view = run(
            directory.path(),
            &["inspect", "--spec=ed.spec", "--editable", "--all"],
        );
        quiet_success(&view);
        let document: toml::Table =
            toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
        assert_eq!(
            document["sources"]["0"]["sha256"].as_str(),
            Some(digest.as_str())
        );
        let checked = run(
            directory.path(),
            &["edit", "--spec=ed.spec", "--all", "--check"],
        );
        assert_eq!(checked.status.code(), Some(1), "{checked:?}");
        assert!(String::from_utf8_lossy(&checked.stderr).contains("64 hexadecimal digits"));
        assert_file(
            directory.path().join("ed.spec"),
            &(SPEC.replace(HASH, &digest)),
        );
        fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
        rejected(
            &run(
                directory.path(),
                &[
                    "edit",
                    "--spec=ed.spec",
                    "--set",
                    &format!("sources.0.sha256={digest}"),
                    "--apply",
                ],
            ),
            "64 hexadecimal digits",
        );
        assert_file(directory.path().join("ed.spec"), SPEC);
    }
    let directory = tempfile::tempdir().unwrap();
    let source = SPEC.replace('\n', "\r\n");
    fs::write(directory.path().join("ed.spec"), &source).unwrap();
    quiet_success(&run(directory.path(), &["check", "--spec=ed.spec"]));
    assert_file(directory.path().join("ed.spec"), &source);
}

#[test]
fn views_preserve_raw_context_and_updates_require_available_values() {
    let directory = tempfile::tempdir().unwrap();
    let line = "URL:            https://www.gnu.org/software/ed/\n";
    let spec = SPEC
        .replace(line, "")
        .replace(URL, "%{url}/%{name}-%{version}.tar.lz");
    let later = spec.replace("%description", &format!("{line}\n%description"));
    fs::write(directory.path().join("ed.spec"), &later).unwrap();
    quiet_success(&run(
        directory.path(),
        &["inspect", "--spec=ed.spec", "--editable", "--all"],
    ));
    fs::write(directory.path().join("ed.spec"), &spec).unwrap();
    quiet_success(&run(
        directory.path(),
        &["inspect", "--spec=ed.spec", "--editable", "--all"],
    ));
    rejected(
        &run(
            directory.path(),
            &[
                "edit",
                "--spec=ed.spec",
                "--set",
                "sources.0.url=%{url}/%{name}-%{version}.tar.lz",
                "--stdout",
            ],
        ),
        "unavailable",
    );
    let explicit = spec.replace(
        "%{url}/%{name}-%{version}.tar.lz",
        "https://example.org/file.tar.lz",
    );
    fs::write(directory.path().join("ed.spec"), &explicit).unwrap();
    quiet_success(&run(
        directory.path(),
        &["inspect", "--spec=ed.spec", "--editable", "--all"],
    ));
}

#[test]
fn source_context_never_executes_unknown_or_dynamic_package_values() {
    let directory = tempfile::tempdir().unwrap();
    for version in ["%{unknown}", "%41", "%(touch MUST_NOT_EXIST)"] {
        let spec = SPEC.replace(
            "Version:        1.22.5",
            &format!("Version:        {version}"),
        );
        let _ = fs::remove_dir_all(directory.path().join(".ruyipack-stage"));
        fs::write(directory.path().join("ed.spec"), spec).unwrap();
        quiet_success(&run(
            directory.path(),
            &["inspect", "--spec=ed.spec", "--editable", "--all"],
        ));
        rejected(
            &run(
                directory.path(),
                &[
                    "edit",
                    "--spec=ed.spec",
                    "--set",
                    &format!("sources.0.url={URL}"),
                    "--stdout",
                ],
            ),
            "sources.0.url",
        );
        assert!(!directory.path().join("MUST_NOT_EXIST").exists());
    }
}

#[test]
fn remote_asset_digests_stay_bound_to_the_adjacent_source_identity() {
    let directory = tempfile::tempdir().unwrap();
    let digest = "AB".repeat(32);
    let manifest = format!(
        "{MANIFEST}\n[sources.2]\nurl = \"https://example.org/second.tar.lz\"\nsha256 = \"{digest}\"\n"
    );
    authoring_workspace(directory.path(), "review", "ed", &manifest);
    let generated = run(
        directory.path(),
        &["gen", "review", "--offline", "--stdout"],
    );
    quiet_success(&generated);
    fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
    let view = run(
        directory.path(),
        &["inspect", "--spec=ed.spec", "--editable", "--all"],
    );
    quiet_success(&view);
    let document: toml::Table = toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
    assert_eq!(document["sources"]["0"]["sha256"].as_str(), Some(HASH));
    assert_eq!(
        document["sources"]["2"]["sha256"].as_str(),
        Some(digest.as_str())
    );
    let edited = run(
        directory.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            &format!("sources.2.sha256={}", "0".repeat(64)),
            "--stdout",
        ],
    );
    success(&edited);
    assert_eq!(
        edited.stdout,
        String::from_utf8(generated.stdout)
            .unwrap()
            .replace(&digest, &"0".repeat(64))
            .as_bytes()
    );
    for spec in [
        SPEC.replace("#!RemoteAsset:", "#RemoteAsset:"),
        SPEC.replace("Source0:", "\nSource0:"),
        SPEC.replace(
            "Source0:",
            &format!("#!RemoteAsset:  sha256:{HASH}\nSource0:"),
        ),
    ] {
        let _ = fs::remove_dir_all(directory.path().join(".ruyipack-stage"));
        fs::write(directory.path().join("ed.spec"), &spec).unwrap();
        rejected(
            &run(
                directory.path(),
                &["inspect", "--spec=ed.spec", "--editable", "--all"],
            ),
            "RemoteAsset",
        );
        assert_file(directory.path().join("ed.spec"), &spec);
    }
}

#[test]
fn authoring_rejects_url_credentials_without_echoing_them() {
    let directory = tempfile::tempdir().unwrap();
    success(&run(directory.path(), &["init"]));
    fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
    for url in [
        "https://demo:private-marker@example.org/archive.tar.gz",
        "https://private-marker@example.org/archive.tar.gz",
        "https://:private-marker@example.org/archive.tar.gz",
    ] {
        for field in ["package.url", "sources.0.url"] {
            let output = run(
                directory.path(),
                &[
                    "edit",
                    "--spec=ed.spec",
                    "--set",
                    &format!("{field}={url}"),
                    "--stdout",
                ],
            );
            rejected(&output, "URL credentials are not allowed");
            assert!(!String::from_utf8_lossy(&output.stderr).contains("private-marker"));
        }
        for old in ["https://www.gnu.org/software/ed/", URL] {
            authoring_workspace(
                directory.path(),
                "review",
                "ed",
                &MANIFEST.replace(old, url),
            );
            let output = run(directory.path(), &["gen", "review", "--stdout"]);
            rejected(&output, "URL credentials are not allowed");
            assert!(!String::from_utf8_lossy(&output.stderr).contains("private-marker"));
            assert!(!directory.path().join("ed.spec.new").exists());
        }
    }
    assert_file(directory.path().join("ed.spec"), SPEC);
}

#[test]
fn unselected_credential_urls_do_not_block_version_edits_or_prevent_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let credential = "https://demo:private-marker@example.org/archive.tar.gz";
    let source = SPEC
        .replace(URL, credential)
        .replace("https://www.gnu.org/software/ed/", credential);
    fs::write(directory.path().join("ed.spec"), &source).unwrap();
    let output = run(
        directory.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            "package.version=2",
            "--stdout",
        ],
    );
    reviewed(&output, "package.version");
    assert_eq!(
        output.stdout,
        source
            .replace("Version:        1.22.5", "Version:        2")
            .as_bytes()
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-marker"));
    let repaired = run(
        directory.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            "package.url=https://example.org/",
            "--set",
            "sources.0.url=https://example.org/archive.tar.gz",
            "--stdout",
        ],
    );
    reviewed(&repaired, "sources.0.url");
    assert!(!String::from_utf8_lossy(&repaired.stdout).contains("private-marker"));
    assert_file(directory.path().join("ed.spec"), &source);
}
