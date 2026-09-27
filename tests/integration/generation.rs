// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box checks for manifest selection, input validation, and rendered facts.

mod build;
mod packages;

use super::support::{assert_file, quiet_success as success, run};

use std::fs;

const MANIFEST: &str = include_str!("../../examples/ed/ed.toml");
const SPEC: &str = include_str!("../fixtures/ed.spec");

fn workspace(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("ed.toml"), source).unwrap();
    directory
}

#[track_caller]
fn renders(source: &str, expected: &str) {
    let directory = workspace(source);
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    assert_eq!(output.stdout, expected.as_bytes());
}

fn rejected(source: &str, message: &str) {
    assert!(
        !message.is_empty(),
        "a rejection must identify its diagnostic"
    );
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
    assert_file(directory.path().join("ed.toml"), source);
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
fn toml_layout_does_not_change_the_fixed_spec() {
    let source = MANIFEST.replace("[build]\nsystem = \"autotools\"\n", "");
    renders(
        &format!("build = {{\n  # TOML 1.1\n  system = \"autotools\",\n}}\n{source}"),
        SPEC,
    );
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
fn vcs_choices_generate_distinct_repository_declarations() {
    for (choice, expected) in [
        (
            "git = \"https://example.org/project.git\"",
            "VCS:            git:https://example.org/project.git\n",
        ),
        (
            "git = \"https://example.org/project\"\nno-public-repository = false",
            "VCS:            git:https://example.org/project\n",
        ),
        ("same-as-url = true", ""),
    ] {
        let source = MANIFEST.replace("no-public-repository = true", choice);
        renders(
            &source,
            &SPEC.replace("# VCS: No VCS link available\n", expected),
        );
    }
}

#[test]
fn vcs_rejects_conflicting_or_invalid_assertions_without_guessing() {
    for choice in [
        "same-as-url = true\nno-public-repository = true",
        "git = \"https://example.org/project\"\nsame-as-url = true",
        "git = \"https://example.org/project\"\nno-public-repository = true",
        "git = \"https://example.org/project\"\nsame-as-url = true\nno-public-repository = true",
        "git = \"\"",
        "git = \"\"\nno-public-repository = true",
        "git = \"http://example.org/project\"",
        "git = \"git:https://example.org/project\"",
        "git = \"https:/example.org/project\"",
        "git = \"https://example.org/project\\nVersion: 2\"",
    ] {
        rejected(
            &MANIFEST.replace("no-public-repository = true", choice),
            "package.vcs",
        );
    }
    rejected(
        &MANIFEST.replace("no-public-repository = true", "same-as-url = \"true\""),
        "expected a boolean",
    );
    rejected(
        &MANIFEST.replace(
            "no-public-repository = true",
            "gti = \"https://example.org/project\"",
        ),
        "unknown field `gti`",
    );
}

#[test]
fn unconfirmed_vcs_is_a_warning_not_an_absence_assertion() {
    let source = MANIFEST.replace("no-public-repository = true", "");
    let directory = workspace(&source);
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("repository status is unconfirmed"));
    assert_eq!(
        output.stdout,
        SPEC.replace("# VCS: No VCS link available\n", "")
            .as_bytes()
    );
    let output = run(
        directory.path(),
        &["gen", "ed", "--check", "--format", "json"],
    );
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["authoring_warnings"][0]
            .as_str()
            .unwrap()
            .contains("package.vcs")
    );
    assert_file(directory.path().join("ed.toml"), &source);
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
    // Empty is invalid too: only an absent sha256 key permits a bare marker.
    for hash in ["", "bad"] {
        rejected(
            &format!("{MANIFEST}{}", extra("1", hash)),
            "sources.1.sha256",
        );
    }
    rejected(
        &format!("{MANIFEST}{}", extra("extra", &"a".repeat(64))),
        "material number",
    );
    rejected(
        &format!(
            "{MANIFEST}{}{}",
            extra("1", &"a".repeat(64)),
            extra("01", &"b".repeat(64))
        ),
        "duplicate material number 1",
    );
}

#[test]
fn malformed_generated_text_cannot_be_published_even_with_force() {
    for (before, after, message) in [
        (
            "A line-oriented text editor",
            "An %{unfinished",
            "parser diagnostics",
        ),
        (
            "\"%{_bindir}/%{name}\"",
            "\"%{_bindir\"",
            "package.files.entries",
        ),
        ("\"lzip\"", "\"lzip >=\"", "parser diagnostics"),
    ] {
        let source = MANIFEST.replace(before, after);
        rejected(&source, message);
        let directory = workspace(&source);
        let path = directory.path().join("ed.spec");
        fs::write(&path, "# maintained by hand\n").unwrap();
        let output = run(directory.path(), &["gen", "ed", "--force"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_file(path, "# maintained by hand\n");
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
        assert_file(path, &source);
        assert!(!directory.path().join("ed.spec").exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn generation_reports_identify_real_inputs_and_never_publish() {
    use sha2::{Digest, Sha256};
    let directory = workspace(MANIFEST);
    let target = directory.path().join("ed.spec");
    fs::write(&target, "existing manual SPEC\n").unwrap();
    let args = ["gen", "ed", "--check", "--format", "json"];
    let first = run(directory.path(), &args);
    success(&first);
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["format_version"], 2);
    assert_eq!(report["scope"], "manifest-generation-static");
    assert_eq!(report["valid"], true);
    assert_eq!(report["manifest"]["display_path"], "ed.toml");
    assert_eq!(
        report["manifest"]["sha256"],
        format!("{:x}", Sha256::digest(MANIFEST.as_bytes()))
    );
    assert_eq!(
        report["profile"]["sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../../profiles/openruyi/profile.toml"))
        )
    );
    assert_eq!(
        report["build_contract"]["name"],
        "openruyi/buildsystems/autotools"
    );
    assert_eq!(
        report["build_contract"]["sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../profiles/openruyi/buildsystems/autotools.toml"
            ))
        )
    );
    assert_eq!(report["report_subject"], "candidate");
    assert_eq!(
        report["report"]["input"]["sha256"],
        format!("{:x}", Sha256::digest(SPEC.as_bytes()))
    );
    assert_eq!(report["report"]["format_version"], 2);
    assert_eq!(report["report"]["evidence"]["stage"], "spec-static");
    assert!(report.get("environment").is_none());
    assert_file(&target, "existing manual SPEC\n");
    assert_file(directory.path().join("ed.toml"), MANIFEST);
    let human = run(directory.path(), &["gen", "ed", "--check"]);
    success(&human);
    assert!(human.stdout.is_empty());
    let plain = MANIFEST.replace("system = \"autotools\"\n", "");
    fs::write(directory.path().join("ed.toml"), plain).unwrap();
    let result = run(directory.path(), &args);
    success(&result);
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(report["build_contract"].is_null());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn generation_check_reports_static_failure_without_writing() {
    let manifest = MANIFEST.replace("GPL-3.0-or-later AND LGPL-2.1-or-later", "Invalid-License");
    let directory = workspace(&manifest);
    let result = run(
        directory.path(),
        &["gen", "ed", "--check", "--format", "json"],
    );
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert_eq!(report["report"]["evidence"]["status"], "fail");
    assert!(!directory.path().join("ed.spec").exists());
    assert_file(directory.path().join("ed.toml"), &manifest);
}

#[test]
fn generation_input_failures_report_the_manifest_without_claiming_a_candidate() {
    use super::support::json_line;
    for source in [
        "not valid TOML!".to_owned(),
        MANIFEST.replace("system = \"autotools\"", "system = \"unknown\""),
        MANIFEST.replace("version = \"1.22.5\"", "version = \"\""),
        MANIFEST.replace(
            "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
            "not-a-url",
        ),
    ] {
        let directory = workspace(&source);
        let result = run(
            directory.path(),
            &["gen", "ed", "--check", "--format", "json"],
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stderr.is_empty(), "{result:?}");
        let report = json_line(&result);
        assert_eq!(report["valid"], false);
        assert!(report["report"].is_null());
        assert!(report["report_subject"].is_null());
        assert!(!report["error"].as_str().unwrap().is_empty());
        assert_eq!(report["manifest"]["display_path"], "ed.toml");
        assert!(report["manifest"]["sha256"].is_string());
        assert_file(directory.path().join("ed.toml"), &source);
        assert!(!directory.path().join("ed.spec").exists());
    }
    let directory = tempfile::tempdir().unwrap();
    let result = run(
        directory.path(),
        &["gen", "ed", "--check", "--format", "json"],
    );
    assert_eq!(result.status.code(), Some(1));
    let report = json_line(&result);
    assert!(report["manifest"]["sha256"].is_null());
    assert!(report["report"].is_null());
}

#[test]
fn generation_and_editing_use_only_their_selected_authority() {
    let directory = workspace("invalid neighboring TOML!");
    let target = directory.path().join("ed.spec");
    fs::write(&target, SPEC).unwrap();
    let edit = run(
        directory.path(),
        &["edit", "ed.spec", "--set", "package.version=2"],
    );
    assert!(edit.status.success(), "{edit:?}");
    let edited = fs::read_to_string(&target).unwrap();
    assert_eq!(
        edited,
        SPEC.replace("Version:        1.22.5", "Version:        2")
    );
    assert_file(
        directory.path().join("ed.toml"),
        "invalid neighboring TOML!",
    );
    fs::write(directory.path().join("ed.toml"), MANIFEST).unwrap();
    let generated = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&generated);
    assert_eq!(generated.stdout, SPEC.as_bytes());
    let conflict = run(directory.path(), &["gen", "ed"]);
    assert_eq!(conflict.status.code(), Some(1));
    assert_file(&target, &edited);
    assert_file(directory.path().join("ed.toml"), MANIFEST);
}

#[test]
fn materials_and_file_lists_compose_without_opening_local_inputs() {
    let source = MANIFEST.replace("[package.files]", "[package.files]\nlists = [\"%{name}.lang\", \"generated.files\"]")
        .replace("entries = [", "entries = [\"%config(noreplace) /etc/ed.conf\", \"%dir %{_datadir}/ed\", \"%ghost %attr(0644,root,root) /var/log/ed.log\", \"%exclude %{_bindir}/unused\", \"%defattr(-,root,root,-)\",");
    let source = format!(
        "{source}\n[sources.1]\npath = \"ed.conf\"\n[patches.20]\npath = \"2000-first.patch\"\n[patches.0]\npath = \"fix-build.patch\"\n"
    );
    let dir = workspace(&source);
    let output = run(dir.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    let spec = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Source1:        ed.conf\n",
        "Patch0:         fix-build.patch\n",
        "%files -f %{name}.lang -f generated.files\n",
        "%config(noreplace) /etc/ed.conf\n",
        "%exclude %{_bindir}/unused\n",
        "%defattr(-,root,root,-)\n",
    ] {
        assert!(spec.contains(expected), "{expected}: {spec}");
    }
    assert_eq!(spec.matches("#!RemoteAsset").count(), 1);
    assert!(spec.find("Patch20:").unwrap() < spec.find("Patch0:").unwrap());
    // No source, patch, or generated list is required on the author's machine.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    for material in [
        "path = \"a.patch\"\nurl = \"https://example.org/a\"",
        "path = \"a.patch\"\nsha256 = \"bad\"",
        "path = \"../a.patch\"",
    ] {
        rejected(
            &format!("{MANIFEST}\n[patches.0]\n{material}\n"),
            "failed to generate SPEC",
        );
    }
    for (row, reason) in [
        ("%dir", "file directive requires a path"),
        ("%config(bogus) /etc/ed", "file directive flags"),
        (
            "%config(noreplace missingok) /etc/ed",
            "file directive flags",
        ),
        ("%verify(not,md5) /etc/ed", "file directive flags"),
        ("%verify(size not md5) /etc/ed", "file directive flags"),
        ("%unknown %{_bindir}/ed", "unsupported file directive"),
        ("%files other", "unsupported file directive"),
        ("%post", "unsupported file directive"),
        (
            "%exclude relative",
            "file paths must start with / or %{ (except doc/license)",
        ),
    ] {
        let invalid = MANIFEST.replace("\"%{_bindir}/%{name}\"", &format!("{row:?}"));
        rejected(&invalid, &format!("package.files.entries: {reason}"));
    }
    rejected(
        &MANIFEST.replace("[package.files]", "[package.files]\nlists = [\"-n\"]"),
        "package.files.lists",
    );
}
