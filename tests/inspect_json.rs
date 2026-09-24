// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for read-only JSON inspection.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

mod support;

fn command(path: &Path) -> Command {
    let mut command = support::command();
    command.arg("inspect").arg(path).args(["--format", "json"]);
    command
}

fn report(output: &Output) -> Value {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    support::json_line(output)
}

#[test]
fn ed_inspection_preserves_syntax_locations_and_input_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ed.spec");
    let source = include_str!("fixtures/ed.spec");
    fs::write(&path, source).unwrap();
    let first = command(&path).output().unwrap();
    let result = report(&first);
    assert_eq!(
        result["input"]["display_path"],
        path.to_string_lossy().as_ref()
    );
    assert_eq!(
        result["input"]["sha256"],
        format!("{:x}", Sha256::digest(source))
    );
    assert_eq!(result["format_version"], 1);
    assert_eq!(
        result["parser"],
        json!({"version": "0.4.1", "revision": "2d5139521c8bef5cf61cb9692d75fd0b5282ffd9"})
    );
    assert_eq!(result["parser_diagnostics"], json!([]));
    let items = result["preamble"].as_array().unwrap();
    assert_eq!(items.len(), 13);
    let version = &items[1]["Preamble"];
    assert_eq!(version["tag"], "Version");
    assert_eq!(
        version["value"],
        json!({"Text": {"segments": [{"Literal": "1.22.5"}]}})
    );
    let span = &version["data"];
    let start = usize::try_from(span["start_byte"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(span["end_byte"].as_u64().unwrap()).unwrap();
    assert_eq!(&source[start..end], "Version:        1.22.5\n");
    assert_eq!(items[6]["Preamble"]["tag"], json!({"Source": 0}));
    assert_eq!(
        items[6]["Preamble"]["value"]["Text"]["segments"][1]["Macro"]["name"],
        "version"
    );

    // Both output routes must carry the same preamble facts.
    let view = rpm_spec::ast::SpecFile {
        items: serde_json::from_value(result["preamble"].clone()).unwrap(),
        data: rpm_spec::ast::Span::default(),
    };
    let config = rpm_spec::printer::PrinterConfig::default().with_preamble_value_column(None);
    let human = support::command()
        .arg("inspect")
        .arg(&path)
        .output()
        .unwrap();
    assert!(human.status.success());
    assert!(human.stderr.is_empty());
    assert_eq!(
        human.stdout,
        rpm_spec::printer::print_with(&view, &config).as_bytes()
    );
    assert_eq!(first.stdout, command(&path).output().unwrap().stdout);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn conditions_and_repeated_tags_are_not_resolved_or_collapsed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conditional.spec");
    let source = "Name: demo\nSummary(fr): Démonstration à 100%%\n%if 0\nVersion: 1\n%elif 1\n# filter this branch too\nVersion: 2\n%else\n# and the otherwise branch\nVersion: %{upstream_version}\n%endif\n%if 0\n# empty after filtering\n%else\n%if 1\nRelease: 1\n%else\n# nested empty branch\n%endif\n%endif\n%if 0\n# entirely tagless\n%endif\nVersion: 3\n%build\ncat <<EOF\nName: not-a-tag\nEOF\n";
    fs::write(&path, source).unwrap();
    let output = command(&path).output().unwrap();
    let result = report(&output);
    let items = result["preamble"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    let summary = &items[1]["Preamble"];
    assert_eq!(summary["lang"], "fr");
    assert_eq!(
        summary["value"]["Text"]["segments"][0]["Literal"],
        "Démonstration à 100%"
    );
    let start = usize::try_from(summary["data"]["start_byte"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(summary["data"]["end_byte"].as_u64().unwrap()).unwrap();
    assert_eq!(&source[start..end], "Summary(fr): Démonstration à 100%%\n");
    let conditional = &items[2]["Conditional"];
    assert_eq!(conditional["branches"].as_array().unwrap().len(), 2);
    for (index, literal) in ["1", "2"].iter().enumerate() {
        let branch = &conditional["branches"][index];
        assert_eq!(
            branch["body"][0]["Preamble"]["value"]["Text"]["segments"][0]["Literal"],
            *literal
        );
    }
    assert_eq!(
        conditional["otherwise"][0]["Preamble"]["value"]["Text"]["segments"][0]["Macro"]["name"],
        "upstream_version"
    );
    let nested = &items[3]["Conditional"];
    assert_eq!(nested["branches"][0]["body"], json!([]));
    let nested = &nested["otherwise"][0]["Conditional"];
    assert_eq!(
        nested["branches"][0]["body"][0]["Preamble"]["tag"],
        "Release"
    );
    assert_eq!(nested["otherwise"], json!([]));
    assert_eq!(
        items[4]["Preamble"]["value"]["Text"]["segments"][0]["Literal"],
        "3"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
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
        let inspected = report(&command(&path).output().unwrap());
        let checked = support::command()
            .arg("check")
            .arg(&path)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert_eq!(checked.status.code(), Some(1));
        assert!(checked.stderr.is_empty());
        let checked: Value = serde_json::from_slice(&checked.stdout).unwrap();
        assert_eq!(
            inspected["parser_diagnostics"],
            checked["parser_diagnostics"]
        );
        assert_eq!(inspected["parser_diagnostics"][0]["severity"], severity);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn text_diagnostics_do_not_present_body_local_offsets_as_source_locations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("diagnostic.spec");
    let source = include_str!("fixtures/ed.spec")
        .replace("Version:        1.22.5", "Version:        %{unfinished")
        .replace(
            "Summary:        A line-oriented text editor",
            "Summary:        Démo \\\n  %{unfinished\nGroup:          100% %{} %{shrink\nAutoReq:        invalid",
        );
    fs::write(&path, &source).unwrap();
    let inspected = report(&command(&path).output().unwrap());
    let checked = support::command()
        .arg("check")
        .arg(&path)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(checked.status.success(), "{checked:?}");
    assert!(checked.stderr.is_empty());
    let checked = support::json_line(&checked);
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
        let matching: Vec<_> = diagnostics.iter().filter(|d| d["code"] == code).collect();
        assert!(!matching.is_empty(), "missing {code}: {diagnostics:?}");
        for diagnostic in matching {
            assert_eq!(diagnostic["severity"], "warning");
            assert!(diagnostic["span"].is_null(), "{diagnostic}");
        }
    }

    // A source-anchored diagnostic and AST node still slice the original bytes.
    let boolean = diagnostics
        .iter()
        .find(|d| d["code"] == "rpmspec/W0017")
        .unwrap();
    let span = &boolean["span"];
    let start = span["start_byte"].as_u64().unwrap() as usize;
    let end = span["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], "AutoReq:        invalid\n");
    let boolean_line = source[..start].lines().count() + 1;
    assert_eq!(span["start_line"], boolean_line);
    let version = &inspected["preamble"][1]["Preamble"]["data"];
    let start = version["start_byte"].as_u64().unwrap() as usize;
    let end = version["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], "Version:        %{unfinished\n");
    for action in ["inspect", "check"] {
        let human = support::command().arg(action).arg(&path).output().unwrap();
        assert!(human.status.success(), "{human:?}");
        let stderr = support::output_text(&human.stderr);
        for code in text_codes {
            assert!(
                stderr
                    .lines()
                    .any(|line| line.starts_with(&format!("{}: warning[{code}]:", path.display()))),
                "{stderr}"
            );
            assert!(
                !stderr.contains(&format!("warning[{code}] at ")),
                "{stderr}"
            );
        }
        assert!(
            stderr.contains(&format!(
                "{}:{boolean_line}:1: warning[rpmspec/W0017]:",
                path.display()
            )),
            "{stderr}"
        );
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}

#[test]
fn empty_and_unreadable_inputs_keep_distinct_results() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.spec");
    fs::write(&path, "# no tags\n").unwrap();
    let result = report(&command(&path).output().unwrap());
    assert_eq!(result["preamble"], json!([]));
    assert_eq!(result["parser_diagnostics"], json!([]));
    fs::write(&path, [0xff]).unwrap();
    for path in [path, directory.path().join("missing.spec")] {
        let output = command(&path).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    assert_eq!(
        fs::read(directory.path().join("input.spec")).unwrap(),
        [0xff]
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}
