// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared Source/RemoteAsset behavior across real generation and editing commands.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const MANIFEST: &str = include_str!("../examples/ed/ed.toml");
const SPEC: &str = include_str!("fixtures/ed.spec");
const URL: &str = "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz";
const HASH: &str = "56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc";

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ruyipack"))
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap()
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

fn rejected(output: &Output, field: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(field),
        "{output:?}"
    );
}

#[test]
fn generated_sources_are_viewable_and_round_trip_without_changing_a_byte() {
    for url in [
        "%{url}/download#/%{name}-%{version}.tar.lz",
        "%url/download#/%name-%version.tar.lz",
        "https://example.org/%{name}/a%%20b-%{version}.tar.lz#/%{name}.tar.lz",
        "https://example.org/a%%2Fb%%3F%%23%%25%%41.tar.lz",
        "https://example.org/archive.tar.lz#/%{name}-%{version}.tar.lz",
    ] {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("ed.toml"), MANIFEST.replace(URL, url)).unwrap();
        let generated = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&generated);
        assert_eq!(generated.stdout, SPEC.replace(URL, url).as_bytes());
        fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
        let view = run(directory.path(), &["edit", "ed.spec", "--view"]);
        success(&view);
        let document: toml::Table =
            toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
        assert_eq!(document["sources"]["0"]["url"].as_str(), Some(url));
        let unchanged = run(
            directory.path(),
            &[
                "edit",
                "ed.spec",
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
fn invalid_or_unsupported_sources_are_rejected_at_both_boundaries_and_on_edit() {
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
        "https://example.org/%{?name}",
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
        fs::write(
            directory.path().join("ed.toml"),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();
        rejected(
            &run(directory.path(), &["gen", "ed", "--stdout"]),
            "sources.0.url",
        );
        let spec = SPEC.replace(URL, url);
        fs::write(directory.path().join("ed.spec"), &spec).unwrap();
        rejected(
            &run(directory.path(), &["edit", "ed.spec", "--view"]),
            "sources.0.url",
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            spec
        );
        fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
        rejected(
            &run(
                directory.path(),
                &["edit", "ed.spec", "--set", &format!("sources.0.url={url}")],
            ),
            "sources.0.url",
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC
        );
        assert!(!directory.path().join("MUST_NOT_EXIST").exists());
    }
}

#[test]
fn existing_http_sources_remain_editable_but_new_generation_requires_https() {
    let url = "http://example.org/%{name}-%{version}.tar.lz";
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.toml"), MANIFEST.replace(URL, url)).unwrap();
    rejected(
        &run(directory.path(), &["gen", "ed", "--stdout"]),
        "require HTTPS",
    );
    let spec = SPEC.replace(URL, url);
    fs::write(directory.path().join("ed.spec"), &spec).unwrap();
    success(&run(directory.path(), &["edit", "ed.spec", "--view"]));
    let edited = run(
        directory.path(),
        &[
            "edit",
            "ed.spec",
            "--set",
            "package.version=1.22.6",
            "--stdout",
        ],
    );
    success(&edited);
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
        fs::write(
            directory.path().join("ed.toml"),
            MANIFEST.replace(HASH, &digest),
        )
        .unwrap();
        let generated = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&generated);
        assert_eq!(generated.stdout, SPEC.replace(HASH, &digest).as_bytes());
        fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
        let view = run(directory.path(), &["edit", "ed.spec", "--view"]);
        success(&view);
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
                "ed.spec",
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
        fs::write(
            directory.path().join("ed.toml"),
            MANIFEST.replace(HASH, &digest),
        )
        .unwrap();
        rejected(
            &run(directory.path(), &["gen", "ed", "--stdout"]),
            "64 hexadecimal digits",
        );
        fs::write(
            directory.path().join("ed.spec"),
            SPEC.replace(HASH, &digest),
        )
        .unwrap();
        rejected(
            &run(directory.path(), &["edit", "ed.spec", "--view"]),
            "64 hexadecimal digits",
        );
        fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
        rejected(
            &run(
                directory.path(),
                &[
                    "edit",
                    "ed.spec",
                    "--set",
                    &format!("sources.0.sha256={digest}"),
                ],
            ),
            "64 hexadecimal digits",
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC
        );
    }
}

#[test]
fn source_context_is_order_independent_and_only_required_when_referenced() {
    let directory = tempfile::tempdir().unwrap();
    let line = "URL:            https://www.gnu.org/software/ed/\n";
    let spec = SPEC
        .replace(line, "")
        .replace(URL, "%{url}/%{name}-%{version}.tar.lz");
    let later = spec.replace("%description", &format!("{line}\n%description"));
    fs::write(directory.path().join("ed.spec"), &later).unwrap();
    success(&run(directory.path(), &["edit", "ed.spec", "--view"]));
    fs::write(directory.path().join("ed.spec"), &spec).unwrap();
    rejected(
        &run(directory.path(), &["edit", "ed.spec", "--view"]),
        "unavailable",
    );
    let explicit = spec.replace(
        "%{url}/%{name}-%{version}.tar.lz",
        "https://example.org/file.tar.lz",
    );
    fs::write(directory.path().join("ed.spec"), &explicit).unwrap();
    success(&run(directory.path(), &["edit", "ed.spec", "--view"]));
}

#[test]
fn source_macros_do_not_recursively_evaluate_package_fields() {
    let directory = tempfile::tempdir().unwrap();
    for version in ["%{unknown}", "%41", "%(touch MUST_NOT_EXIST)"] {
        let spec = SPEC.replace(
            "Version:        1.22.5",
            &format!("Version:        {version}"),
        );
        fs::write(directory.path().join("ed.spec"), spec).unwrap();
        rejected(
            &run(directory.path(), &["edit", "ed.spec", "--view"]),
            "not a supported static literal",
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
    fs::write(directory.path().join("ed.toml"), manifest).unwrap();
    let generated = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&generated);
    fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
    let view = run(directory.path(), &["edit", "ed.spec", "--view"]);
    success(&view);
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
            "ed.spec",
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
        fs::write(directory.path().join("ed.spec"), &spec).unwrap();
        rejected(
            &run(directory.path(), &["edit", "ed.spec", "--view"]),
            "RemoteAsset",
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            spec
        );
    }
}
