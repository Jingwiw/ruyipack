// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box checks for manifest selection, input validation, and rendered facts.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const MANIFEST: &str = include_str!("../examples/ed/ed.toml");
const SPEC: &str = include_str!("fixtures/ed.spec");
const SOURCE: &str = "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz";

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ruyipack"))
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run ruyipack")
}

fn workspace(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.toml"), source).unwrap();
    directory
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn rejected(source: &str, message: &str) {
    let directory = workspace(source);
    let output = run(directory.path(), &["gen", "ed"]);
    assert_eq!(output.status.code(), Some(1), "{message}: {output:?}");
    assert!(output.stdout.is_empty(), "{message}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{output:?}"
    );
    assert!(!directory.path().join("ed.spec").exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.toml")).unwrap(),
        source
    );
}

#[test]
fn selected_manifest_controls_the_package_and_default_destination() {
    let directory = workspace(MANIFEST);
    fs::write(directory.path().join("other.toml"), "not valid TOML").unwrap();
    let output = run(directory.path(), &["gen", "ed"]);
    success(&output);
    assert_eq!(
        fs::read(directory.path().join("ed.spec")).unwrap(),
        SPEC.as_bytes()
    );

    fs::create_dir(directory.path().join("inputs")).unwrap();
    fs::rename(
        directory.path().join("ed.toml"),
        directory.path().join("inputs/recipe.toml"),
    )
    .unwrap();
    let missing = run(directory.path(), &["gen", "ed", "--stdout"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("ed.toml"));
    let explicit = run(
        directory.path(),
        &["gen", "ed", "--manifest", "inputs/recipe.toml"],
    );
    success(&explicit);
    assert_eq!(
        fs::read(directory.path().join("inputs/ed.spec")).unwrap(),
        SPEC.as_bytes()
    );

    let mismatch = run(
        directory.path(),
        &["gen", "other", "--manifest", "inputs/recipe.toml"],
    );
    assert_eq!(mismatch.status.code(), Some(1));
    assert!(mismatch.stdout.is_empty());
    assert!(!directory.path().join("inputs/other.spec").exists());
    assert!(!directory.path().join("other.spec").exists());
}

#[test]
fn package_selector_is_not_a_path() {
    let directory = workspace(MANIFEST);
    for name in ["", ".", "..", "../ed", "ed/", "/ed", "ed\\other"] {
        let output = run(directory.path(), &["gen", name, "--manifest", "ed.toml"]);
        assert_eq!(output.status.code(), Some(1), "{name:?}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("one filename component"));
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn ed_output_is_fixed_and_independent_of_toml_layout() {
    let directory = workspace(MANIFEST);
    for _ in 0..2 {
        let output = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&output);
        assert_eq!(output.stdout, SPEC.as_bytes());
    }
    let source = MANIFEST.replace("[build]\nsystem = \"autotools\"\n", "");
    let source = format!("build = {{\n  # TOML 1.1\n  system = \"autotools\",\n}}\n{source}");
    fs::write(directory.path().join("ed.toml"), source).unwrap();
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    assert_eq!(output.stdout, SPEC.as_bytes());
}

#[test]
fn changed_input_values_change_the_rendered_facts() {
    let source = MANIFEST
        .replace("name = \"ed\"", "name = \"sample\"")
        .replace("1.22.5", "2.0~rc1")
        .replace("A line-oriented text editor", "Small text utility")
        .replace("\"autoconf\"", "\"autoconf >= 2.71\"")
        .replace("\"lzip\"", "\"lzip\", \"pkgconfig(libcurl) >= 8\"")
        .replace("\"COPYING\"", "\"LICENSE\"")
        .replace("\"%{_bindir}/%{name}\"", "\"%{_bindir}/sample\"");
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("sample.toml"), source).unwrap();
    let output = run(directory.path(), &["gen", "sample", "--stdout"]);
    success(&output);
    let expected = SPEC
        .replace("Name:           ed", "Name:           sample")
        .replace("1.22.5", "2.0~rc1")
        .replace("A line-oriented text editor", "Small text utility")
        .replace(
            "BuildRequires:  autoconf\n",
            "BuildRequires:  autoconf >= 2.71\n",
        )
        .replace(
            "BuildRequires:  lzip\n",
            "BuildRequires:  lzip\nBuildRequires:  pkgconfig(libcurl) >= 8\n",
        )
        .replace("%license COPYING", "%license LICENSE")
        .replace("%{_bindir}/%{name}\n", "%{_bindir}/sample\n");
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn required_and_invalid_fields_are_rejected_before_publication() {
    for (before, after, message) in [
        ("version = \"1.22.5\"\n", "", "missing field `version`"),
        ("1.22.5", "%{release_version}", "package.version"),
        ("name = \"ed\"", "name = \"../ed\"", "package.name"),
        (
            "copyright-years = \"2025\"",
            "copyright-years = \"2026-2025\"",
            "spec.copyright-years",
        ),
        ("A line-oriented text editor", "", "package.summary"),
        (
            "GPL-3.0-or-later AND LGPL-2.1-or-later",
            "MIT AND",
            "package.license",
        ),
        (
            "https://www.gnu.org/software/ed/",
            "https:/www.gnu.org/software/ed/",
            "package.url",
        ),
        (
            "system = \"autotools\"",
            "system = \"unknown\"",
            "build.system",
        ),
        ("\"autoconf\", ", "", "build-requires.rpm"),
        (
            "\"%{_bindir}/%{name}\"",
            "\"relative/path\"",
            "package.files.entries",
        ),
        (
            "version = \"1.22.5\"",
            "version = \"1.22.5\"\nunknown = true",
            "unknown field `unknown`",
        ),
    ] {
        rejected(&MANIFEST.replace(before, after), message);
    }
}

#[test]
fn sources_keep_macros_rename_fragments_and_encoded_paths() {
    for url in [
        "%{url}/download#/%{name}-%{version}.tar.lz",
        "%url/download#/%name-%version.tar.lz",
        "https://example.org/a%20b.tar.lz",
    ] {
        let directory = workspace(&MANIFEST.replace(SOURCE, url));
        let output = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&output);
        assert_eq!(output.stdout, SPEC.replace(SOURCE, url).as_bytes());
    }
    for url in [
        "https:/example.org/ed.tar.lz",
        "https://example.org/a b.tar.lz",
        "https://example.org/%{version.tar.lz",
        "https://example.org/%{release_tag}.tar.lz",
        "https://example.org/%{?version}.tar.lz",
        "https://example.org/%{lua:print(123)}.tar.lz",
        "https://example.org/%(touch MUST_NOT_EXIST).tar.lz",
    ] {
        rejected(&MANIFEST.replace(SOURCE, url), "sources.0.url");
    }
}

#[test]
fn numbered_sources_keep_their_own_checksums_and_numeric_order() {
    let extra = |number: &str, hash: &str| {
        format!(
            "\n[sources.{number}]\nurl = \"https://example.org/extra-{number}.tar.lz\"\nsha256 = \"{hash}\"\n"
        )
    };
    let second = extra("2", &"b".repeat(64));
    let tenth = extra("10", &"a".repeat(64));
    let directory = workspace(&format!("{MANIFEST}{tenth}{second}"));
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    let expected = SPEC.replace("BuildSystem:", &format!(
        "#!RemoteAsset:  sha256:{}\nSource2:        https://example.org/extra-2.tar.lz\n\
         #!RemoteAsset:  sha256:{}\nSource10:       https://example.org/extra-10.tar.lz\nBuildSystem:",
        "b".repeat(64), "a".repeat(64),
    ));
    assert_eq!(output.stdout, expected.as_bytes());

    rejected(&MANIFEST.replace("[sources.0]", "[sources.1]"), "sources.0");
    rejected(
        &format!("{MANIFEST}{}", extra("1", "bad")),
        "sources.1.sha256",
    );
    rejected(
        &format!("{MANIFEST}{}", extra("extra", &"a".repeat(64))),
        "source number",
    );
    rejected(
        &format!(
            "{MANIFEST}{}{}",
            extra("1", &"a".repeat(64)),
            extra("01", &"b".repeat(64))
        ),
        "duplicate source number 1",
    );
}

#[test]
fn malformed_generated_text_cannot_be_published_even_with_force() {
    for (before, after) in [
        ("A line-oriented text editor", "An %{unfinished"),
        ("\"%{_bindir}/%{name}\"", "\"%{_bindir\""),
        ("\"lzip\"", "\"lzip >=\""),
    ] {
        let source = MANIFEST.replace(before, after);
        rejected(&source, "parser diagnostics");
        let directory = workspace(&source);
        let path = directory.path().join("ed.spec");
        fs::write(&path, "# maintained by hand\n").unwrap();
        let output = run(directory.path(), &["gen", "ed", "--force"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(fs::read_to_string(path).unwrap(), "# maintained by hand\n");
    }
}

#[test]
fn generation_errors_identify_the_selected_manifest_without_writing() {
    for (source, message) in [
        ("[package".to_owned(), "TOML parse error"),
        (
            MANIFEST.replace("A line-oriented text editor", ""),
            "package.summary",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let manifest = "ed input-编辑器.toml";
        let path = directory.path().join(manifest);
        fs::write(&path, &source).unwrap();

        let output = run(directory.path(), &["gen", "ed", "--manifest", manifest]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(&format!("failed to generate SPEC from {manifest}:")),
            "{error}"
        );
        assert!(error.contains(message), "{error}");
        assert_eq!(fs::read_to_string(path).unwrap(), source);
        assert!(!directory.path().join("ed.spec").exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
