// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source context and field selection compose without broadening the edit boundary.

use super::super::support::{assert_file, rejected, success};
use super::{SPEC, command, fixture, inspect, preview};
use std::{fmt::Write as _, fs, path::Path, process::Output};

const URL: &str = "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz";
const HASH: &str = "56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc";

fn run(directory: &Path, args: &[&str]) -> Output {
    command(directory).args(args).output().unwrap()
}

fn selected_view(directory: &Path, field: &str) -> Output {
    inspect(directory)
        .args(["--spec=ed.spec", "--field", field])
        .output()
        .unwrap()
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
fn source_context_accepts_known_ordered_values_and_rejects_ambiguity() {
    for (version, known) in [
        ("Version: 1\n%if 0\nVersion: 2\n%endif", true),
        ("Version: 1\n%if 0\n%if 1\nVersion: 2\n%endif\n%endif", true),
        ("%if 0\nVersion: 2\n%endif\nVersion: 1", true),
        ("Version: 1\nVersion: 2\nVersion: 3", false),
        ("%global version 9\nVersion: 1", true),
        ("Version: 1\n%global version 9", true),
        ("Version: 1\n%if %{unknown}\nVersion: 2\n%endif", false),
    ] {
        let source = SPEC.replace("Version:        1.22.5", version);
        let directory = fixture(&source);
        only_source_url(&selected_view(directory.path(), "sources.0.url"), URL);
        let output = run(
            directory.path(),
            &[
                "--spec=ed.spec",
                "--set",
                &format!("sources.0.url={URL}"),
                "--stdout",
            ],
        );
        if known {
            success(&output);
            assert_eq!(output.stdout, source.as_bytes());
        } else {
            rejected(&output, "sources.0.url");
        }
        let summary = selected_view(directory.path(), "package.summary");
        success(&summary);
        assert!(
            String::from_utf8(summary.stdout)
                .unwrap()
                .contains("A line-oriented text editor")
        );
        let literal = source.replace(URL, "https://example.org/archive.tar.lz");
        fs::write(directory.path().join("ed.spec"), &literal).unwrap();
        // This case deliberately starts a new baseline after changing fixture bytes.
        fs::remove_dir_all(directory.path().join(".ruyipack-stage")).unwrap();
        only_source_url(
            &selected_view(directory.path(), "sources.0.url"),
            "https://example.org/archive.tar.lz",
        );
        assert_file(directory.path().join("ed.spec"), &literal);
    }
}

#[test]
fn unknown_include_and_statement_invalidate_context_not_literal_source_urls() {
    for source in ["%include absent-context.inc\n", "%{unresolved_statement}\n"]
        .into_iter()
        .flat_map(|directive| {
            [
                format!("{directive}{SPEC}"),
                SPEC.replace("#!RemoteAsset:", &format!("{directive}#!RemoteAsset:")),
            ]
        })
    {
        let directory = fixture(&source);
        only_source_url(&selected_view(directory.path(), "sources.0.url"), URL);
        rejected(
            &run(
                directory.path(),
                &[
                    "--spec=ed.spec",
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
        // This case deliberately starts a new baseline after changing fixture bytes.
        fs::remove_dir_all(directory.path().join(".ruyipack-stage")).unwrap();
        only_source_url(
            &selected_view(directory.path(), "sources.0.url"),
            "https://example.org/archive.tar.lz",
        );
        let unchanged = run(
            directory.path(),
            &[
                "--spec=ed.spec",
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
                    "--spec=ed.spec",
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
                    "--spec=ed.spec",
                    "--set",
                    "package.version=2",
                    "--set",
                    "sources.0.url=https://example.org/%{version}.tar.lz",
                    "--stdout",
                ],
            ),
            "unavailable or ambiguous",
        );
        assert_file(directory.path().join("ed.spec"), &literal);
    }
}

#[test]
fn an_unambiguous_package_context_is_used_without_entering_the_selected_document() {
    let directory = fixture(SPEC);
    only_source_url(&selected_view(directory.path(), "sources.0.url"), URL);
    let replacement = "%{url}/%{name}-%{version}.tar.lz";
    preview(
        directory.path(),
        &format!("sources.0.url={replacement}"),
        &SPEC.replace(URL, replacement),
    );
    assert_file(directory.path().join("ed.spec"), SPEC);
}

#[test]
fn a_source_url_draft_cannot_add_checksum_or_package_context_fields() {
    let directory = fixture(SPEC);
    success(&run(
        directory.path(),
        &[
            "--spec=ed.spec",
            "--field",
            "sources.0.url",
            "--prepare",
            "drafts",
        ],
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
            &["--from", "drafts", "--check", "--format", "toml"],
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let report = super::support::machine_report(&output);
        assert!(report.get("valid").is_none());
        assert_file(directory.path().join("ed.spec"), SPEC);
    }
    fs::write(&draft, allowed).unwrap();
    let output = run(directory.path(), &["--from", "drafts", "--stdout"]);
    success(&output);
    assert_eq!(output.stdout, SPEC.replace(URL, replacement).as_bytes());
}

#[test]
fn selecting_one_source_url_preserves_an_unmapped_sibling_source_and_its_digest() {
    let source = SPEC.replace(
        "BuildSystem:",
        "#!RemoteAsset:  sha256:not-a-digest\nSource2:        %{unknown}\nBuildSystem:",
    );
    let directory = fixture(&source);
    let replacement = "https://example.org/archive.tar.lz";
    preview(
        directory.path(),
        &format!("sources.0.url={replacement}"),
        &source.replace(URL, replacement),
    );
    assert_file(directory.path().join("ed.spec"), &source);
}

#[test]
fn implicit_source_numbers_follow_rpm_before_field_selection() {
    for case in include_str!("../../fixtures/source-numbering.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let (headers, numbers) = case.split_once('\t').unwrap();
        let headers = headers.split(',').collect::<Vec<_>>();
        let numbers = numbers
            .split(',')
            .map(|n| n.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        let mut declarations = String::new();
        for (i, header) in headers.iter().enumerate() {
            writeln!(
                declarations,
                "#!RemoteAsset\n{header}: https://example.org/asset-{i}.tar.gz"
            )
            .unwrap();
        }
        let old = format!("#!RemoteAsset:  sha256:{HASH}\nSource0:        {URL}\n");
        let source = format!(
            "%global _smp_mflags -j1\n{}",
            SPEC.replace(&old, &declarations)
        );
        let directory = fixture(&source);
        let mut expected = source.clone();
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
                    "--spec=ed.spec",
                    "--set",
                    &format!("{field}=https://example.org/replaced.tar.gz"),
                    "--stdout",
                ],
            );
            success(&edited);
            // Earlier pending field values remain in the same persistent stage.
            expected = expected.replace(
                &format!("https://example.org/asset-{i}.tar.gz"),
                "https://example.org/replaced.tar.gz",
            );
            assert_eq!(edited.stdout, expected.as_bytes());
        }
        if !numbers.contains(&0) {
            rejected(
                &selected_view(directory.path(), "sources.0"),
                "unknown field",
            );
        }
        assert_file(directory.path().join("ed.spec"), &source);
    }
}

#[test]
fn uncertain_implicit_source_numbers_do_not_block_unrelated_edits() {
    for prefix in [
        "%if %{unknown}\nSource3: https://example.org/conditional.tar.gz\n%endif\n",
        "%include absent.inc\n",
        "%{unknown_statement}\n",
        "%global number %{unknown_number}\n",
        "Vendor: %{unknown_vendor}\n",
    ] {
        let source = format!("{prefix}{}", SPEC.replace("Source0:", "Source:"));
        let directory = fixture(&source);
        rejected(&selected_view(directory.path(), "sources.0.url"), "sources");
        rejected(
            &run(
                directory.path(),
                &[
                    "--spec=ed.spec",
                    "--set",
                    "sources.0.url=https://example.org/replaced.tar.gz",
                ],
            ),
            "sources",
        );
        let version = run(
            directory.path(),
            &["--spec=ed.spec", "--set", "package.version=2", "--stdout"],
        );
        success(&version);
        assert_eq!(
            version.stdout,
            source
                .replace("Version:        1.22.5", "Version:        2")
                .as_bytes()
        );
        assert_file(directory.path().join("ed.spec"), &source);
    }
}

#[test]
fn digest_whitespace_has_one_meaning_without_rewriting_untouched_bytes() {
    let replacement = "a".repeat(64);
    let changed = SPEC.replace(HASH, &replacement);
    for digest in [
        format!(" {HASH}"),
        format!("{HASH} "),
        format!("\t{HASH}\t"),
    ] {
        let source = SPEC.replace(HASH, &digest);
        let directory = fixture(&source);
        success(&super::super::support::run(
            directory.path(),
            &["check", "--spec=ed.spec", "--policy", "submit"],
        ));
        for (value, expected) in [(HASH, &source), (replacement.as_str(), &changed)] {
            let assignment = format!("sources.0.sha256={value}");
            let output = run(
                directory.path(),
                &["--spec=ed.spec", "--set", &assignment, "--stdout"],
            );
            success(&output);
            assert_eq!(output.stdout, expected.as_bytes());
        }
        assert_file(directory.path().join("ed.spec"), &source);
    }
}

#[test]
fn a_damaged_selected_digest_can_be_repaired_without_changing_other_bytes() {
    let replacement = "a".repeat(64);
    let damaged = "INVALID";
    let source = SPEC.replace(HASH, damaged);
    let directory = fixture(&source);
    let view = selected_view(directory.path(), "sources.0.sha256");
    success(&view);
    let document: toml::Table = toml::from_str(std::str::from_utf8(&view.stdout).unwrap()).unwrap();
    assert_eq!(document["sources"]["0"]["sha256"].as_str(), Some(damaged));
    for invalid in [damaged, "still-invalid"] {
        rejected(
            &run(
                directory.path(),
                &[
                    "--spec=ed.spec",
                    "--set",
                    &format!("sources.0.sha256={invalid}"),
                    "--apply",
                ],
            ),
            "expected 64 hexadecimal digits",
        );
        assert_file(directory.path().join("ed.spec"), &source);
    }
    success(&run(
        directory.path(),
        &[
            "--spec=ed.spec",
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
    assert_file(directory.path().join("ed.spec"), &source);
    success(&run(directory.path(), &["--from", "drafts", "--apply"]));
    assert_file(
        directory.path().join("ed.spec"),
        &(SPEC.replace(HASH, &replacement)),
    );
}

#[test]
fn a_source_url_edit_preserves_its_unselected_damaged_digest() {
    let source = SPEC.replace(HASH, "INVALID");
    let directory = fixture(&source);
    let replacement = "https://example.org/replacement.tar.lz";
    let output = preview(
        directory.path(),
        &format!("sources.0.url={replacement}"),
        &source.replace(URL, replacement),
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("review required"));
    assert_file(directory.path().join("ed.spec"), &source);
}

#[test]
fn mapping_errors_identify_the_construct_without_blocking_unrelated_fields() {
    for (source, field, message) in [
        (
            SPEC.replace(
                "BuildSystem:",
                "# Local packaging file\nSource1: libev.pc\nBuildSystem:",
            ),
            "sources",
            "sources.1 (Source1): no adjacent RemoteAsset",
        ),
        (
            SPEC.replace(
                "BuildSystem:",
                "BuildOption(conf): --enable-foo\nBuildSystem:",
            ),
            "",
            "BuildOption(conf)",
        ),
        (
            SPEC.replace("%files", "%files -f %{pyproject_files}"),
            "package.files",
            "%files -f %{pyproject_files}",
        ),
    ] {
        let directory = fixture(&source);
        let args = if field.is_empty() {
            vec!["--spec=ed.spec", "--all"]
        } else {
            vec!["--spec=ed.spec", "--field", field]
        };
        let output = inspect(directory.path()).args(args).output().unwrap();
        rejected(&output, message);
        assert!(String::from_utf8_lossy(&output.stderr).contains("--field"));
        success(&selected_view(directory.path(), "package.version"));
        assert_file(directory.path().join("ed.spec"), &source);
    }
}
