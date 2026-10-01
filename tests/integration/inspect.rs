// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for read-only TOML inspection.

use super::support::{assert_file, success};

use sha2::{Digest, Sha256};
use std::{fs, process::Output};
use toml::Value;

use super::support;

fn report(output: &Output) -> Value {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    support::machine_report(output)
}

#[test]
fn ed_inspection_preserves_syntax_locations_and_input_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ed.spec");
    let source = include_str!("../fixtures/ed.spec");
    fs::write(&path, source).unwrap();
    let first = support::spec_report("inspect", &path);
    let result = report(&first);
    assert_eq!(
        result["input"]["display_path"].as_str(),
        Some(path.to_string_lossy().as_ref())
    );
    assert_eq!(
        result["input"]["sha256"].as_str(),
        Some((format!("{:x}", Sha256::digest(source))).as_str())
    );
    assert_eq!(result["format_version"].as_integer(), Some(2));
    assert_eq!(result["parser_diagnostics"], toml::Value::Array(vec![]));
    let items = result["preamble"].as_array().unwrap();
    assert_eq!(items.len(), 13);
    let version = &items[1];
    assert_eq!(version["tag"]["name"].as_str(), Some("Version"));
    assert_eq!(
        version["value"],
        toml::Value::Table(toml::toml! {
            Text = { segments = [{ Literal = "1.22.5" }] }
        })
    );
    let span = &version["span"];
    let start = usize::try_from(span["start_byte"].as_integer().unwrap()).unwrap();
    let end = usize::try_from(span["end_byte"].as_integer().unwrap()).unwrap();
    assert_eq!(&source[start..end], "Version:        1.22.5\n");
    assert_eq!(items[6]["tag"]["name"].as_str(), Some("Source"));
    assert_eq!(items[6]["tag"]["number"].as_integer(), Some(0));
    assert_eq!(version["raw"].as_str(), Some(&source[start..end]));
    assert_eq!(
        items[6]["value"]["Text"]["segments"][1]["Macro"]["name"].as_str(),
        Some("version")
    );

    // The human printer and machine projection preserve the same observed facts,
    // but machine records are not a round-trip upstream AST.
    let human = support::command()
        .arg("inspect")
        .arg("--spec")
        .arg(&path)
        .output()
        .unwrap();
    assert!(human.status.success());
    assert!(human.stderr.is_empty());
    assert_eq!(
        support::output_text(&human.stdout),
        "Name: ed\nVersion: 1.22.5\nRelease: %autorelease\nSummary: A line-oriented text editor\nLicense: GPL-3.0-or-later AND LGPL-2.1-or-later\nURL: https://www.gnu.org/software/ed/\nSource0: https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz\nBuildSystem: autotools\nBuildRequires: autoconf\nBuildRequires: automake\nBuildRequires: libtool\nBuildRequires: make\nBuildRequires: lzip\n"
    );
    assert_file(&path, source);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn conditions_and_repeated_tags_are_not_resolved_or_collapsed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conditional.spec");
    let source = "Name: demo\nSummary(fr): Démonstration à 100%%\n%if 0\nVersion: 1\n%elif 1\n# filter this branch too\nVersion: 2\n%else\n# and the otherwise branch\nVersion: %{upstream_version}\n%endif\n%if 0\n# empty after filtering\n%else\n%if 1\nRelease: 1\n%else\n# nested empty branch\n%endif\n%endif\n%if 0\n# entirely tagless\n%endif\nVersion: 3\n%build\ncat <<EOF\nName: not-a-tag\nEOF\n";
    fs::write(&path, source).unwrap();
    let output = support::spec_report("inspect", &path);
    let result = report(&output);
    let items = result["preamble"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    let summary = &items[1];
    assert_eq!(summary["lang"].as_str(), Some("fr"));
    assert_eq!(
        summary["value"]["Text"]["segments"][0]["Literal"].as_str(),
        Some("Démonstration à 100%")
    );
    let start = usize::try_from(summary["span"]["start_byte"].as_integer().unwrap()).unwrap();
    let end = usize::try_from(summary["span"]["end_byte"].as_integer().unwrap()).unwrap();
    assert_eq!(&source[start..end], "Summary(fr): Démonstration à 100%%\n");
    let conditional = &items[2];
    assert_eq!(conditional["branches"].as_array().unwrap().len(), 2);
    for (index, literal) in ["1", "2"].iter().enumerate() {
        let branch = &conditional["branches"][index];
        assert_eq!(
            branch["body"][0]["value"]["Text"]["segments"][0]["Literal"].as_str(),
            Some(*literal)
        );
    }
    assert_eq!(
        conditional["otherwise"][0]["value"]["Text"]["segments"][0]["Macro"]["name"].as_str(),
        Some("upstream_version")
    );
    let nested = &items[3];
    assert_eq!(nested["branches"][0]["body"], toml::Value::Array(vec![]));
    let nested = &nested["otherwise"][0];
    assert_eq!(
        nested["branches"][0]["body"][0]["tag"]["name"].as_str(),
        Some("Release")
    );
    assert_eq!(nested["otherwise"], toml::Value::Array(vec![]));
    assert_eq!(
        items[4]["value"]["Text"]["segments"][0]["Literal"].as_str(),
        Some("3")
    );
    assert_file(&path, source);
}

#[test]
fn inspection_and_check_share_complete_parser_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("diagnostic.spec");
    for (source, severity) in [
        ("Name: demo\nAutoReq: invalid\n", "warning"),
        ("Name: demo\n%package\n", "error"),
    ] {
        fs::write(&path, source).unwrap();
        let inspected = report(&support::spec_report("inspect", &path));
        let checked = support::spec_report("check", &path);
        assert_eq!(checked.status.code(), Some(1));
        assert!(checked.stderr.is_empty());
        let checked = super::support::machine_report(&checked);
        assert_eq!(
            inspected["parser_diagnostics"],
            checked["parser_diagnostics"]
        );
        assert_eq!(
            inspected["parser_diagnostics"][0]["severity"].as_str(),
            Some(severity)
        );
        assert_file(&path, source);
    }
}

#[test]
fn text_diagnostics_do_not_present_body_local_offsets_as_source_locations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("diagnostic.spec");
    let source = include_str!("../fixtures/ed.spec")
        .replace("Version:        1.22.5", "Version:        %{unfinished")
        .replace(
            "Summary:        A line-oriented text editor",
            "Summary:        Démo \\\n  %{unfinished\nGroup:          100% %{} %{shrink\nAutoReq:        invalid",
        );
    fs::write(&path, &source).unwrap();
    let inspected = report(&support::spec_report("inspect", &path));
    let checked = support::spec_report("check", &path);
    assert!(checked.status.success(), "{checked:?}");
    assert!(checked.stderr.is_empty());
    let checked = support::machine_report(&checked);
    assert_eq!(
        inspected["parser_diagnostics"],
        checked["parser_diagnostics"]
    );
    let diagnostics = inspected["parser_diagnostics"].as_array().unwrap();
    let text_codes = [
        "rpmspec/W0001",
        "rpmspec/W0003",
        "rpmspec/W0004",
        "rpmspec/W0021",
    ];
    for code in text_codes {
        let matching: Vec<_> = diagnostics
            .iter()
            .filter(|d| d["code"].as_str() == Some(code))
            .collect();
        assert!(!matching.is_empty(), "missing {code}: {diagnostics:?}");
        for diagnostic in matching {
            assert_eq!(diagnostic["severity"].as_str(), Some("warning"));
            assert!(diagnostic.get("span").is_none(), "{diagnostic}");
        }
    }

    // A source-anchored diagnostic and inspection record still slice the original bytes.
    let boolean = diagnostics
        .iter()
        .find(|d| d["code"].as_str() == Some("rpmspec/W0017"))
        .unwrap();
    let span = &boolean["span"];
    let start = usize::try_from(span["start_byte"].as_integer().unwrap()).unwrap();
    let end = usize::try_from(span["end_byte"].as_integer().unwrap()).unwrap();
    assert_eq!(&source[start..end], "AutoReq:        invalid\n");
    let boolean_line = source[..start].lines().count() + 1;
    assert_eq!(
        span["start_line"].as_integer(),
        Some(i64::try_from(boolean_line).unwrap())
    );
    let version = &inspected["preamble"][1]["span"];
    let start = usize::try_from(version["start_byte"].as_integer().unwrap()).unwrap();
    let end = usize::try_from(version["end_byte"].as_integer().unwrap()).unwrap();
    assert_eq!(&source[start..end], "Version:        %{unfinished\n");
    for action in ["inspect", "check"] {
        let human = support::command()
            .current_dir(directory.path())
            .arg(action)
            .arg("--spec")
            .arg(&path)
            .output()
            .unwrap();
        assert!(human.status.success(), "{human:?}");
        let stderr = support::output_text(&human.stderr);
        for code in text_codes {
            assert!(
                stderr
                    .lines()
                    .any(|line| line.starts_with(&format!("[WARN] spec [{code}]:"))),
                "{stderr}"
            );
        }
        assert!(
            stderr.contains(&format!("[WARN] spec[{boolean_line}:1] [rpmspec/W0017]:")),
            "{stderr}"
        );
    }
    assert_file(&path, &source);
}

#[test]
fn empty_input_has_no_inspect_facts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.spec");
    fs::write(&path, "# no tags\n").unwrap();
    let result = report(&support::spec_report("inspect", &path));
    assert_eq!(result["preamble"], toml::Value::Array(vec![]));
    assert_eq!(result["parser_diagnostics"], toml::Value::Array(vec![]));
    assert_file(&path, "# no tags\n");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

const SPEC: &str = include_str!("../fixtures/ed.spec");

#[test]
fn full_view_exposes_existing_fields_without_writing_files() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
    let first = support::run(
        directory.path(),
        &["inspect", "--spec=ed.spec", "--all", "--editable"],
    );
    success(&first);
    assert!(first.stderr.is_empty());
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&first.stdout).unwrap()).unwrap();
    let expected: toml::Table = toml::from_str(include_str!("../fixtures/ed.edit.toml")).unwrap();
    assert_eq!(document, expected);
    assert_file(directory.path().join("ed.spec"), SPEC);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn selected_view_and_schema_have_the_same_narrow_shape() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
    let output = support::run(
        directory.path(),
        &[
            "inspect",
            "--spec=ed.spec",
            "--editable",
            "--field",
            "package.version",
        ],
    );
    success(&output);
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(document.len(), 1);
    assert_eq!(document["package"].as_table().unwrap().len(), 1);
    let output = support::run(
        directory.path(),
        &[
            "schema",
            "edit",
            "--spec=ed.spec",
            "--field",
            "package.version",
        ],
    );
    success(&output);
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    let package = &schema["properties"]["package"];
    assert_eq!(package["additionalProperties"], false);
    assert_eq!(package["properties"].as_object().unwrap().len(), 1);
    assert_eq!(
        package["properties"]["version"]["type"].as_str(),
        Some("string")
    );
    assert!(
        package["properties"]["version"]["description"]
            .as_str()
            .unwrap()
            .contains("Version")
    );
    assert_file(directory.path().join("ed.spec"), SPEC);
}

#[test]
fn group_selection_keeps_descendants_without_duplicating_overlaps() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.spec"), SPEC).unwrap();
    let output = support::run(
        directory.path(),
        &[
            "inspect",
            "--spec=ed.spec",
            "--editable",
            "--field",
            "package.files",
            "--field",
            "package.files.doc",
        ],
    );
    success(&output);
    let document: toml::Table =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(document["package"].as_table().unwrap().len(), 1);
    let files = document["package"]["files"].as_table().unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        files.contains_key("doc") && files.contains_key("license") && files.contains_key("entries")
    );
    let bad = support::run(
        directory.path(),
        &[
            "inspect",
            "--spec=ed.spec",
            "--editable",
            "--field",
            "package.unknown",
        ],
    );
    assert_eq!(bad.status.code(), Some(1), "{bad:?}");
    assert!(bad.stdout.is_empty());
}

#[test]
fn inspection_distinguishes_implicit_and_explicit_source_patch_numbers_in_conditions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("numbers.spec");
    let source = "Name: demo\nSource: https://example.invalid/implicit.tar\nSource0: https://example.invalid/zero.tar\nSource3: https://example.invalid/three.tar\nPatch: implicit.patch\nPatch0: zero.patch\n%if 0\nSource7: https://example.invalid/conditional.tar\n%else\nPatch9: conditional.patch\n%endif\n%build\nSource: body-not-a-tag\n";
    fs::write(&path, source).unwrap();
    let result = report(&support::spec_report("inspect", &path));
    assert_eq!(result["scope"].as_str(), Some("main-package-syntax"));
    assert_eq!(
        result["not_checked"]
            .clone()
            .try_into::<Vec<String>>()
            .unwrap(),
        ["macro-expansion", "native-rpm", "build"]
    );
    let items = result["preamble"].as_array().unwrap();
    assert_eq!(items.len(), 7);
    for (index, name, number, raw) in [
        (
            1,
            "Source",
            None,
            "Source: https://example.invalid/implicit.tar\n",
        ),
        (
            2,
            "Source",
            Some(0),
            "Source0: https://example.invalid/zero.tar\n",
        ),
        (
            3,
            "Source",
            Some(3),
            "Source3: https://example.invalid/three.tar\n",
        ),
        (4, "Patch", None, "Patch: implicit.patch\n"),
        (5, "Patch", Some(0), "Patch0: zero.patch\n"),
    ] {
        let tag = &items[index];
        assert_eq!(tag["kind"].as_str(), Some("tag"));
        assert_eq!(tag["tag"]["name"].as_str(), Some(name));
        assert_eq!(tag["tag"].get("number").and_then(Value::as_integer), number);
        assert_eq!(tag["raw"].as_str(), Some(raw));
        let span = &tag["span"];
        let start = usize::try_from(span["start_byte"].as_integer().unwrap()).unwrap();
        let end = usize::try_from(span["end_byte"].as_integer().unwrap()).unwrap();
        assert_eq!(&source[start..end], raw);
    }
    let conditional = &items[6];
    assert_eq!(conditional["kind"].as_str(), Some("conditional"));
    let branch = &conditional["branches"][0];
    assert_eq!(branch["header"].as_str(), Some("%if 0\n"));
    assert_eq!(branch["body"][0]["tag"]["number"].as_integer(), Some(7));
    assert_eq!(
        conditional["otherwise"][0]["tag"]["name"].as_str(),
        Some("Patch")
    );
    assert_eq!(
        conditional["otherwise"][0]["tag"]["number"].as_integer(),
        Some(9)
    );
    assert_file(&path, source);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}
