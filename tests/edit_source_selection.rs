// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source context and field selection compose without broadening the edit boundary.

use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

const SPEC: &str = include_str!("fixtures/ed.spec");
const URL: &str = "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz";
const HASH: &str = "56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc";

fn fixture(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), source).unwrap();
    directory
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ruyipack"))
        .arg("edit")
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn rejected(output: &Output, message: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{output:?}"
    );
}

fn selected_view(directory: &Path, field: &str) -> Output {
    run(directory, &["ed.spec", "--view", "--field", field])
}

fn only_source_url(output: &Output, expected: &str) {
    success(output);
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(document.len(), 1);
    let sources = document["sources"].as_table().unwrap();
    assert_eq!(sources.len(), 1);
    let source = sources["0"].as_table().unwrap();
    assert_eq!(source.len(), 1);
    assert_eq!(source["url"].as_str(), Some(expected));
}

#[test]
fn a_conditional_context_field_blocks_only_sources_that_reference_it() {
    for version in [
        "Version: 1\n%if 0\nVersion: 2\n%endif",
        "Version: 1\n%if 0\n%if 1\nVersion: 2\n%endif\n%endif",
    ] {
        let source = SPEC.replace("Version:        1.22.5", version);
        let directory = fixture(&source);
        only_source_url(&selected_view(directory.path(), "sources.0.url"), URL);
        rejected(
            &run(
                directory.path(),
                &[
                    "ed.spec",
                    "--set",
                    &format!("sources.0.url={URL}"),
                    "--stdout",
                ],
            ),
            "unsupported source macro",
        );
        let summary = selected_view(directory.path(), "package.summary");
        success(&summary);
        assert!(
            String::from_utf8(summary.stdout)
                .unwrap()
                .contains("A line-oriented text editor")
        );
        let literal = source.replace(URL, "https://example.org/archive.tar.lz");
        fs::write(directory.path().join("ed.spec"), &literal).unwrap();
        only_source_url(
            &selected_view(directory.path(), "sources.0.url"),
            "https://example.org/archive.tar.lz",
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            literal
        );
    }
}

#[test]
fn unknown_include_and_statement_invalidate_context_not_literal_source_urls() {
    for directive in ["%include absent-context.inc\n", "%{unresolved_statement}\n"] {
        let source = format!("{directive}{SPEC}");
        let directory = fixture(&source);
        only_source_url(&selected_view(directory.path(), "sources.0.url"), URL);
        rejected(
            &run(
                directory.path(),
                &[
                    "ed.spec",
                    "--set",
                    &format!("sources.0.url={URL}"),
                    "--stdout",
                ],
            ),
            "unavailable or ambiguous",
        );
        success(&selected_view(directory.path(), "package.summary"));
        let literal = source.replace(URL, "https://example.org/archive.tar.lz");
        fs::write(directory.path().join("ed.spec"), &literal).unwrap();
        only_source_url(
            &selected_view(directory.path(), "sources.0.url"),
            "https://example.org/archive.tar.lz",
        );
        let unchanged = run(
            directory.path(),
            &[
                "ed.spec",
                "--set",
                "sources.0.url=https://example.org/archive.tar.lz",
                "--stdout",
            ],
        );
        success(&unchanged);
        assert_eq!(unchanged.stdout, literal.as_bytes());
        rejected(
            &run(
                directory.path(),
                &[
                    "ed.spec",
                    "--set",
                    "sources.0.url=https://example.org/%{version}.tar.lz",
                    "--stdout",
                ],
            ),
            "unavailable or ambiguous",
        );
        rejected(
            &run(
                directory.path(),
                &[
                    "ed.spec",
                    "--set",
                    "package.version=2",
                    "--set",
                    "sources.0.url=https://example.org/%{version}.tar.lz",
                    "--stdout",
                ],
            ),
            "unavailable or ambiguous",
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            literal
        );
    }
}

#[test]
fn an_unambiguous_package_context_is_used_without_entering_the_selected_document() {
    let directory = fixture(SPEC);
    only_source_url(&selected_view(directory.path(), "sources.0.url"), URL);
    let replacement = "%{url}/%{name}-%{version}.tar.lz";
    let output = run(
        directory.path(),
        &[
            "ed.spec",
            "--set",
            &format!("sources.0.url={replacement}"),
            "--stdout",
        ],
    );
    success(&output);
    assert_eq!(output.stdout, SPEC.replace(URL, replacement).as_bytes());
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC
    );
}

#[test]
fn a_source_url_draft_cannot_add_checksum_or_package_context_fields() {
    let directory = fixture(SPEC);
    success(&run(
        directory.path(),
        &["ed.spec", "--field", "sources.0.url", "--prepare", "drafts"],
    ));
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.path().join("drafts/.state/schema/0.json")).unwrap(),
    )
    .unwrap();
    let schema = &schema["properties"]["sources"]["properties"]["0"];
    assert_eq!(schema["required"], serde_json::json!(["url"]));
    assert_eq!(schema["properties"].as_object().unwrap().len(), 1);
    assert_eq!(schema["additionalProperties"], false);
    let draft = directory.path().join("drafts/ed.toml");
    let replacement = "https://example.org/%{name}-%{version}.tar.lz";
    let allowed = format!("[sources.0]\nurl = '{replacement}'\n");
    for changed in [
        format!("{allowed}sha256 = '{}'\n", "0".repeat(64)),
        format!("{allowed}\n[package]\nversion = '2'\n"),
        "[sources.0]\n".into(),
    ] {
        fs::write(&draft, changed).unwrap();
        let output = run(
            directory.path(),
            &["--from", "drafts", "--check", "--format", "json"],
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["valid"], false);
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC
        );
    }
    fs::write(&draft, allowed).unwrap();
    let output = run(directory.path(), &["--from", "drafts", "--stdout"]);
    success(&output);
    assert_eq!(output.stdout, SPEC.replace(URL, replacement).as_bytes());
    assert!(String::from_utf8(output.stdout).unwrap().contains(HASH));
}

#[test]
fn selecting_one_source_url_preserves_an_unmapped_sibling_source_and_its_digest() {
    let source = SPEC.replace(
        "BuildSystem:",
        "#!RemoteAsset:  sha256:not-a-digest\nSource2:        %{unknown}\nBuildSystem:",
    );
    let directory = fixture(&source);
    let replacement = "https://example.org/archive.tar.lz";
    let output = run(
        directory.path(),
        &[
            "ed.spec",
            "--set",
            &format!("sources.0.url={replacement}"),
            "--stdout",
        ],
    );
    success(&output);
    assert_eq!(output.stdout, source.replace(URL, replacement).as_bytes());
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        source
    );
}

#[test]
fn implicit_source_numbers_follow_rpm_before_field_selection() {
    for case in include_str!("fixtures/source-numbering.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let (headers, numbers) = case.split_once('\t').unwrap();
        let headers = headers.split(',').collect::<Vec<_>>();
        let numbers = numbers
            .split(',')
            .map(|n| n.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        let declarations = headers
            .iter()
            .enumerate()
            .map(|(i, header)| {
                format!("#!RemoteAsset\n{header}: https://example.org/asset-{i}.tar.gz\n")
            })
            .collect::<String>();
        let old = format!("#!RemoteAsset:  sha256:{HASH}\nSource0:        {URL}\n");
        let source = format!(
            "%global _smp_mflags -j1\n{}",
            SPEC.replace(&old, &declarations)
        );
        let directory = fixture(&source);
        for (i, number) in numbers.iter().enumerate() {
            let field = format!("sources.{number}.url");
            let view = selected_view(directory.path(), &field);
            success(&view);
            let document: toml::Table =
                toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
            assert_eq!(
                document["sources"][number.to_string()]["url"].as_str(),
                Some(format!("https://example.org/asset-{i}.tar.gz").as_str())
            );
            let edited = run(
                directory.path(),
                &[
                    "ed.spec",
                    "--set",
                    &format!("{field}=https://example.org/replaced.tar.gz"),
                    "--stdout",
                ],
            );
            success(&edited);
            assert_eq!(
                edited.stdout,
                source
                    .replace(
                        &format!("https://example.org/asset-{i}.tar.gz"),
                        "https://example.org/replaced.tar.gz"
                    )
                    .as_bytes()
            );
        }
        if !numbers.contains(&0) {
            rejected(
                &selected_view(directory.path(), "sources.0"),
                "unknown field",
            );
        }
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            source
        );
    }
}

#[test]
fn uncertain_implicit_source_numbers_do_not_block_unrelated_edits() {
    for prefix in [
        "%if 0\nSource3: https://example.org/conditional.tar.gz\n%endif\n",
        "%include absent.inc\n",
        "%{unknown_statement}\n",
        "%global number %{unknown_number}\n",
        "Vendor: %{unknown_vendor}\n",
        "%if 0\nVendor: %{unknown_vendor}\n%endif\n",
    ] {
        let source = format!("{prefix}{}", SPEC.replace("Source0:", "Source:"));
        let directory = fixture(&source);
        rejected(&selected_view(directory.path(), "sources.0.url"), "sources");
        rejected(
            &run(
                directory.path(),
                &[
                    "ed.spec",
                    "--set",
                    "sources.0.url=https://example.org/replaced.tar.gz",
                ],
            ),
            "sources",
        );
        let version = run(
            directory.path(),
            &["ed.spec", "--set", "package.version=2", "--stdout"],
        );
        success(&version);
        assert_eq!(
            version.stdout,
            source
                .replace("Version:        1.22.5", "Version:        2")
                .as_bytes()
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            source
        );
    }
}

#[test]
fn a_damaged_selected_digest_can_be_repaired_without_changing_other_bytes() {
    let replacement = "a".repeat(64);
    for damaged in ["INVALID".to_owned(), "g".repeat(64), "a".repeat(65)] {
        let source = SPEC.replace(HASH, &damaged);
        let directory = fixture(&source);
        let view = selected_view(directory.path(), "sources.0.sha256");
        success(&view);
        let document: toml::Table =
            toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
        assert_eq!(
            document["sources"]["0"]["sha256"].as_str(),
            Some(damaged.as_str())
        );
        for invalid in [damaged.as_str(), "still-invalid"] {
            rejected(
                &run(
                    directory.path(),
                    &["ed.spec", "--set", &format!("sources.0.sha256={invalid}")],
                ),
                "expected 64 hexadecimal digits",
            );
            assert_eq!(
                fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
                source
            );
        }
        success(&run(
            directory.path(),
            &[
                "ed.spec",
                "--field",
                "sources.0.sha256",
                "--prepare",
                "drafts",
            ],
        ));
        fs::write(
            directory.path().join("drafts/ed.toml"),
            format!("[sources.0]\nsha256 = '{replacement}'\n"),
        )
        .unwrap();
        let preview = run(directory.path(), &["--from", "drafts", "--stdout"]);
        success(&preview);
        assert_eq!(preview.stdout, SPEC.replace(HASH, &replacement).as_bytes());
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            source
        );
        success(&run(directory.path(), &["--from", "drafts"]));
        assert_eq!(
            fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
            SPEC.replace(HASH, &replacement)
        );
    }
}

#[test]
fn a_source_url_edit_preserves_its_unselected_damaged_digest() {
    let source = SPEC.replace(HASH, "INVALID");
    let directory = fixture(&source);
    let replacement = "https://example.org/replacement.tar.lz";
    let output = run(
        directory.path(),
        &[
            "ed.spec",
            "--set",
            &format!("sources.0.url={replacement}"),
            "--stdout",
        ],
    );
    success(&output);
    assert_eq!(output.stdout, source.replace(URL, replacement).as_bytes());
    assert!(String::from_utf8_lossy(&output.stderr).contains("review required"));
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        source
    );
}
