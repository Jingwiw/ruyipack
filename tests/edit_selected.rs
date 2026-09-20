// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Selection is a source projection, not a requirement to map the whole SPEC.

use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

const SOURCE: &str = include_str!("fixtures/ed.spec");

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ruyipack"));
    command
        .arg("edit")
        .current_dir(directory)
        .stdin(Stdio::null());
    command
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn fixture(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("case.spec"), source).unwrap();
    directory
}

fn extended() -> String {
    SOURCE
        .replace(
            "Name:           ed",
            concat!(
                "%global local_value preserved\n",
                "%global inert %(touch macro-was-executed)\n",
                "Name:           ed"
            ),
        )
        .replace(
            "URL:            https://www.gnu.org/software/ed/",
            concat!(
                "URL:            https://www.gnu.org/software/ed/\n",
                "VCS:            git:https://example.invalid/ed.git\n",
                "BuildOption(conf): --disable-silent-rules\n",
                "Provides:       base:ed\n",
                "%if 0\nBuildRequires: ignored-conditional\n%endif"
            ),
        )
}

fn view(directory: &Path, field: &str) -> toml::Table {
    let output = command(directory)
        .args(["case.spec", "--field", field, "--view"])
        .output()
        .unwrap();
    success(&output);
    toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap()
}

#[test]
fn selected_version_preserves_unmapped_tags_macros_and_unselected_conditional_bytes() {
    let source = extended();
    assert!(source.contains("VCS:"));
    let directory = fixture(&source);
    let output = command(directory.path())
        .args(["case.spec", "--set", "package.version=1.22.6", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        output.stdout,
        source
            .replace("Version:        1.22.5", "Version:        1.22.6")
            .as_bytes()
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("case.spec")).unwrap(),
        source
    );
    assert!(!directory.path().join("macro-was-executed").exists());
    let document = view(directory.path(), "package.version");
    assert_eq!(
        document,
        toml::from_str::<toml::Table>("[package]\nversion = '1.22.5'\n").unwrap()
    );
    let full = command(directory.path())
        .args(["case.spec", "--view"])
        .output()
        .unwrap();
    assert!(!full.status.success(), "{full:?}");
    assert!(full.stdout.is_empty());
}

#[test]
fn selected_version_ignores_unselected_duplicate_fields_and_unresolved_source_mapping() {
    let source = extended()
        .replace(
            "Summary:        A line-oriented text editor",
            "Summary:        First summary\nSummary:        Second summary",
        )
        .replace(
            "Source0:        https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
            "Source0:        %{unresolved_source}",
        );
    assert!(source.contains("Second summary") && source.contains("%{unresolved_source}"));
    let directory = fixture(&source);
    let output = command(directory.path())
        .args(["case.spec", "--set", "package.version=1.22.6", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        output.stdout,
        source
            .replace("Version:        1.22.5", "Version:        1.22.6")
            .as_bytes()
    );
}

#[test]
fn duplicate_and_conditional_selected_versions_are_rejected_without_output() {
    for version in [
        "Version:        1.22.5\nVersion:        1.22.6",
        "Version:        1.22.5\n%if 0\nVersion: 1.22.6\n%endif",
        "%if 0\nVersion: 1.22.5\n%else\nVersion: 1.22.6\n%endif",
        "%if 0\n%if 1\nVersion: 1.22.5\n%endif\n%endif",
    ] {
        let source = SOURCE.replace("Version:        1.22.5", version);
        let directory = fixture(&source);
        for mode in [
            vec!["--set", "package.version=1.23", "--stdout"],
            vec!["--field", "package.version", "--view"],
        ] {
            let output = command(directory.path())
                .arg("case.spec")
                .args(mode)
                .output()
                .unwrap();
            assert!(!output.status.success(), "{output:?}");
            assert!(output.stdout.is_empty());
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(
                error.contains("duplicate") || error.contains("conditional selection"),
                "{error}"
            );
        }
        assert_eq!(
            fs::read_to_string(directory.path().join("case.spec")).unwrap(),
            source
        );
    }
}

#[test]
fn parser_errors_anywhere_still_block_a_selected_view() {
    let source = format!("%endif\n{SOURCE}");
    let directory = fixture(&source);
    let output = command(directory.path())
        .args(["case.spec", "--field", "package.version", "--view"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("parser errors")
    );
}

#[test]
fn selected_draft_schema_and_resume_keep_selection_and_reject_shape_changes() {
    let source = extended();
    let directory = fixture(&source);
    success(
        &command(directory.path())
            .args([
                "case.spec",
                "--field",
                "package.version",
                "--prepare",
                "drafts",
            ])
            .output()
            .unwrap(),
    );
    let draft = directory.path().join("drafts/case.toml");
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.path().join("drafts/.state/schema/0.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(schema["properties"].as_object().unwrap().len(), 1);
    assert_eq!(
        schema["properties"]["package"]["properties"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        schema["properties"]["package"]["required"],
        serde_json::json!(["version"])
    );
    assert_eq!(
        schema["properties"]["package"]["additionalProperties"],
        false
    );
    let index: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.path().join("drafts/.state/index.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        index["drafts"][0]["fields"],
        serde_json::json!(["package.version"])
    );
    for changed in [
        "[package]\nversion = '1.23'\nname = 'surprise'\n",
        "[package]\n",
    ] {
        fs::write(&draft, changed).unwrap();
        let output = command(directory.path())
            .args(["--from", "drafts", "--check", "--format", "json"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["valid"], false);
        assert_eq!(
            fs::read_to_string(directory.path().join("case.spec")).unwrap(),
            source
        );
    }
    fs::write(&draft, "[package]\nversion = '1.22.6'\n").unwrap();
    success(
        &command(directory.path())
            .args(["--from", "drafts", "--check"])
            .output()
            .unwrap(),
    );
    let output = command(directory.path())
        .args(["--from", "drafts", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        output.stdout,
        source
            .replace("Version:        1.22.5", "Version:        1.22.6")
            .as_bytes()
    );
}

#[test]
fn selecting_one_copyright_leaf_keeps_its_companion_out_of_draft() {
    let source = extended();
    let directory = fixture(&source);
    let document = view(directory.path(), "spec.copyright-years");
    assert_eq!(document["spec"].as_table().unwrap().len(), 1);
    let output = command(directory.path())
        .args([
            "case.spec",
            "--set",
            "spec.copyright-years=2026",
            "--stdout",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        output.stdout,
        source.replace("(C) 2025 ", "(C) 2026 ").as_bytes()
    );
}

#[test]
fn selected_file_array_preserves_unselected_directives_when_cleared() {
    let source = extended().replace("%files\n", "%files\n%config /etc/ed.conf\n");
    let directory = fixture(&source);
    success(
        &command(directory.path())
            .args([
                "case.spec",
                "--field",
                "package.files.doc",
                "--prepare",
                "drafts",
            ])
            .output()
            .unwrap(),
    );
    fs::write(
        directory.path().join("drafts/case.toml"),
        "[package.files]\ndoc = []\n",
    )
    .unwrap();
    let output = command(directory.path())
        .args(["--from", "drafts", "--stdout"])
        .output()
        .unwrap();
    success(&output);
    let expected = source
        .lines()
        .filter(|line| !line.starts_with("%doc "))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(output.stdout, expected.as_bytes());
}
