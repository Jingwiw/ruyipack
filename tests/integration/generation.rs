// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box checks for manifest selection, input validation, and rendered facts.

mod build;
mod packages;

use super::support::{assert_file, authoring_workspace, quiet_success, run};

use std::fs;

const MANIFEST: &str = include_str!("../../examples/ed/ed.toml");
const SPEC: &str = include_str!("../fixtures/ed.spec");

fn workspace(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    authoring_workspace(directory.path(), "authoring", "ed", source);
    directory
}

#[track_caller]
fn renders(source: &str, expected: &str) {
    let directory = workspace(source);
    let output = run(directory.path(), &["gen", "authoring", "--stdout"]);
    quiet_success(&output);
    assert_eq!(output.stdout, expected.as_bytes());
}

fn rejected(source: &str, message: &str) {
    assert!(
        !message.is_empty(),
        "a rejection must identify its diagnostic"
    );
    let directory = workspace(source);
    let output = run(directory.path(), &["gen", "authoring"]);
    assert_eq!(output.status.code(), Some(1), "{message}: {output:?}");
    assert!(output.stdout.is_empty(), "{message}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{output:?}"
    );
    assert!(!directory.path().join("ed.spec").exists());
    assert!(
        !directory
            .path()
            .join("work/authoring/.cache/ed.resolved.toml")
            .exists()
    );
    assert_file(directory.path().join("work/authoring/ed.toml"), source);
}

#[test]
fn work_binding_controls_completion_and_spec_requires_explicit_destination() {
    let directory = workspace(MANIFEST);
    let work = directory.path().join("work/authoring");
    fs::write(work.join("other.toml"), "not valid TOML").unwrap();
    crate::support::success(&run(directory.path(), &["gen", "authoring", "--offline"]));
    assert_file(work.join(".cache/ed.candidate.spec"), SPEC);
    let completed: toml::Value =
        toml::from_str(&fs::read_to_string(work.join(".cache/ed.resolved.toml")).unwrap()).unwrap();
    assert_eq!(
        completed["manifest"]["package"]["noarch"].as_bool(),
        Some(false)
    );
    assert_eq!(completed["role"].as_str(), Some("resolved-authoring"));
    assert_eq!(
        completed["profile"]["release"].as_str(),
        Some("%autorelease")
    );
    assert_eq!(
        completed["manifest"]["sources"][0]["number"].as_integer(),
        Some(0)
    );
    assert_eq!(
        completed["manifest"]["sources"][0]["url"].as_str(),
        Some("https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz")
    );
    assert_eq!(
        completed["spec_sha256"].as_str(),
        Some(format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(SPEC)).as_str())
    );
    assert_file(work.join("ed.toml"), MANIFEST);
    assert!(!work.join("recipe").exists());
    assert!(!work.join("ed.spec").exists());
    crate::support::success(&run(
        directory.path(),
        &["gen", "authoring", "--output=.", "--offline"],
    ));
    assert_file(work.join("ed.spec"), SPEC);
    crate::support::success(&run(
        directory.path(),
        &["gen", "authoring", "--output=review.spec", "--offline"],
    ));
    assert_file(directory.path().join("review.spec"), SPEC);
    let rejected = run(directory.path(), &["gen", "--manifest=ed.toml"]);
    assert_eq!(rejected.status.code(), Some(2));
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
    authoring_workspace(directory.path(), "sample-work", "sample", &source);
    let output = run(directory.path(), &["gen", "sample-work", "--stdout"]);
    quiet_success(&output);
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
    let output = run(directory.path(), &["gen", "authoring", "--stdout"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("repository status is unconfirmed"));
    assert_eq!(
        output.stdout,
        SPEC.replace("# VCS: No VCS link available\n", "")
            .as_bytes()
    );
    let output = run(
        directory.path(),
        &["gen", "authoring", "--check", "--format", "toml"],
    );
    assert!(output.status.success(), "{output:?}");
    let report = super::support::machine_report(&output);
    assert!(
        report["authoring_warnings"][0]
            .as_str()
            .unwrap()
            .contains("package.vcs")
    );
    assert_file(directory.path().join("work/authoring/ed.toml"), &source);
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
    let output = run(directory.path(), &["gen", "authoring", "--stdout"]);
    quiet_success(&output);
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
        let output = run(
            directory.path(),
            &["gen", "authoring", "--output=ed.spec", "--force"],
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_file(path, "# maintained by hand\n");
    }
}

#[test]
fn generation_errors_identify_the_work_input_without_writing() {
    for (source, message) in [
        ("[package".to_owned(), "TOML parse error"),
        (
            MANIFEST.replace("A line-oriented text editor", ""),
            "package.summary",
        ),
    ] {
        let directory = workspace(&source);
        let path = directory.path().join("work/authoring/ed.toml");
        let output = run(directory.path(), &["gen", "authoring"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("failed to generate SPEC from"), "{error}");
        assert!(error.contains("work/authoring/ed.toml"), "{error}");
        assert!(error.contains(message), "{error}");
        assert_file(path, &source);
        assert!(
            !directory
                .path()
                .join("work/authoring/.cache/ed.resolved.toml")
                .exists()
        );
    }
}

#[test]
fn generation_reports_identify_real_inputs_and_never_publish() {
    use sha2::{Digest, Sha256};
    let directory = workspace(MANIFEST);
    let target = directory.path().join("ed.spec");
    fs::write(&target, "existing manual SPEC\n").unwrap();
    let args = ["gen", "authoring", "--check", "--format", "toml"];
    let first = run(directory.path(), &args);
    quiet_success(&first);
    let report = super::support::machine_report(&first);
    assert_eq!(report["format_version"].as_integer(), Some(5));
    assert_eq!(report["scope"].as_str(), Some("manifest-generation-static"));
    assert_eq!(report["valid"].as_bool(), Some(true));
    assert_eq!(
        report["input"]["display_path"].as_str(),
        Some(
            directory
                .path()
                .canonicalize()
                .unwrap()
                .join("work/authoring/ed.toml")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        report["input"]["sha256"].as_str(),
        Some((format!("{:x}", Sha256::digest(MANIFEST.as_bytes()))).as_str())
    );
    assert_eq!(
        report["profile"]["sha256"].as_str(),
        Some(
            (format!(
                "{:x}",
                Sha256::digest(include_bytes!("../../profiles/openruyi/profile.toml"))
            ))
            .as_str()
        )
    );
    assert_eq!(
        report["build_contract"]["name"].as_str(),
        Some("openruyi/buildsystems/autotools")
    );
    assert_eq!(
        report["build_contract"]["sha256"].as_str(),
        Some(
            (format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../profiles/openruyi/buildsystems/autotools.toml"
                ))
            ))
            .as_str()
        )
    );
    assert_eq!(
        report["report"]["input"]["sha256"].as_str(),
        Some((format!("{:x}", Sha256::digest(SPEC.as_bytes()))).as_str())
    );
    assert_eq!(report["report"]["format_version"].as_integer(), Some(2));
    assert_eq!(
        report["report"]["evidence"]["stage"].as_str(),
        Some("spec-static")
    );
    assert!(report.get("environment").is_none());
    assert_file(&target, "existing manual SPEC\n");
    assert_file(directory.path().join("work/authoring/ed.toml"), MANIFEST);
    let human = run(directory.path(), &["gen", "authoring", "--check"]);
    quiet_success(&human);
    assert!(human.stdout.is_empty());
    let plain = MANIFEST.replace("system = \"autotools\"\n", "");
    fs::write(directory.path().join("work/authoring/ed.toml"), plain).unwrap();
    let result = run(directory.path(), &args);
    quiet_success(&result);
    let report = super::support::machine_report(&result);
    assert!(report.get("build_contract").is_none());
    assert!(
        !directory
            .path()
            .join("work/authoring/.cache/ed.resolved.toml")
            .exists()
    );
}

#[test]
fn generation_check_reports_static_failure_without_writing() {
    let manifest = MANIFEST.replace("GPL-3.0-or-later AND LGPL-2.1-or-later", "Invalid-License");
    let directory = workspace(&manifest);
    for mode in [
        vec!["--check"],
        vec!["--diff"],
        vec!["--diff", "--check"],
        vec!["--diff", "--apply", "--force"],
    ] {
        let mut args = vec!["gen", "authoring", "--offline", "--format", "toml"];
        args.extend(&mode);
        let result = run(directory.path(), &args);
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        let report = super::support::machine_report(&result);
        assert_eq!(report["valid"].as_bool(), Some(false));
        assert_eq!(
            report["report"]["evidence"]["status"].as_str(),
            Some("fail")
        );
        assert!(report["written"].as_array().unwrap().is_empty());
        if mode.contains(&"--diff") {
            assert!(report["diff"].as_str().unwrap().contains("+License:"));
        }
        let work = directory.path().join("work/authoring");
        assert!(!work.join("recipe").exists());
        assert!(!work.join(".cache/ed.resolved.toml").exists());
        assert_file(work.join("ed.toml"), &manifest);
    }
}

#[test]
fn generation_input_failures_report_known_scope_without_claiming_a_candidate() {
    use super::support::machine_report;
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
            &["gen", "authoring", "--check", "--format", "toml"],
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stderr.is_empty(), "{result:?}");
        let report = machine_report(&result);
        assert_eq!(report["scope"].as_str(), Some("manifest-generation-static"));
        assert!(report.get("valid").is_none());
        assert_eq!(report["success"].as_bool(), Some(false));
        assert!(report.get("report").is_none());
        assert_eq!(report["error"]["code"].as_str(), Some("generation-failed"));
        assert!(!report["error"]["message"].as_str().unwrap().is_empty());
        assert_eq!(
            report["input"]["display_path"].as_str(),
            Some(
                directory
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("work/authoring/ed.toml")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(report["input"]["sha256"].is_str());
        assert_file(directory.path().join("work/authoring/ed.toml"), &source);
        assert!(!directory.path().join("ed.spec").exists());
    }
    let directory = tempfile::tempdir().unwrap();
    let result = run(
        directory.path(),
        &["gen", "authoring", "--check", "--format", "toml"],
    );
    assert_eq!(result.status.code(), Some(1));
    let report = machine_report(&result);
    assert!(report["input"].get("sha256").is_none());
    assert!(report.get("report").is_none());
    let directory = tempfile::tempdir().unwrap();
    let result = run(
        directory.path(),
        &["gen", "missing", "--check", "--format=toml"],
    );
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        machine_report(&result)["scope"].as_str(),
        Some("generation-static")
    );
}

#[test]
fn generation_and_editing_use_only_their_selected_authority() {
    let directory = workspace("invalid neighboring TOML!");
    let target = directory.path().join("ed.spec");
    fs::write(&target, SPEC).unwrap();
    let edit = run(
        directory.path(),
        &[
            "edit",
            "--spec=ed.spec",
            "--set",
            "package.version=2",
            "--apply",
        ],
    );
    assert!(edit.status.success(), "{edit:?}");
    let edited = fs::read_to_string(&target).unwrap();
    assert_eq!(
        edited,
        SPEC.replace("Version:        1.22.5", "Version:        2")
    );
    assert_file(
        directory.path().join("work/authoring/ed.toml"),
        "invalid neighboring TOML!",
    );
    fs::write(directory.path().join("work/authoring/ed.toml"), MANIFEST).unwrap();
    let generated = run(directory.path(), &["gen", "authoring", "--stdout"]);
    quiet_success(&generated);
    assert_eq!(generated.stdout, SPEC.as_bytes());
    let conflict = run(directory.path(), &["gen", "authoring", "--output=ed.spec"]);
    assert_eq!(conflict.status.code(), Some(1));
    assert_file(&target, &edited);
    assert_file(directory.path().join("work/authoring/ed.toml"), MANIFEST);
}

#[test]
fn materials_and_file_lists_compose_without_opening_local_inputs() {
    let source = MANIFEST.replace("[package.files]", "[package.files]\nlists = [\"%{name}.lang\", \"generated.files\"]")
        .replace("entries = [", "entries = [\"%config(noreplace) /etc/ed.conf\", \"%dir %{_datadir}/ed\", \"%ghost %attr(0644,root,root) /var/log/ed.log\", \"%exclude %{_bindir}/unused\", \"%defattr(-,root,root,-)\",");
    let source = format!(
        "{source}\n[sources.1]\npath = \"ed.conf\"\n[patches.20]\npath = \"2000-first.patch\"\n[patches.0]\npath = \"fix-build.patch\"\n"
    );
    let dir = workspace(&source);
    let output = run(dir.path(), &["gen", "authoring", "--stdout"]);
    quiet_success(&output);
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
    assert!(
        !dir.path()
            .join("work/authoring/.cache/ed.resolved.toml")
            .exists()
    );
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

#[test]
fn selected_edit_input_retains_unmapped_script_bytes() {
    use super::support::success;
    let directory = tempfile::tempdir().unwrap();
    let source = SPEC.replace(
        "%files\n",
        "%prep\nprintf '%s\\n' 'keep this exact custom script'\n\n%files\n",
    );
    let source = source.replace(
        "BuildSystem:",
        "%if %{unavailable}\nSource: opaque.tar\n%endif\nSource: local.tar\nBuildSystem:",
    );
    let work = super::support::recipe_workspace(directory.path(), "authoring", "ed", &source);
    success(&run(
        directory.path(),
        &[
            "edit",
            "authoring",
            "--field",
            "package.summary",
            "--prepare",
            "work/authoring",
        ],
    ));
    let edited = "[package]\nsummary = 'Changed summary'\n";
    fs::write(work.join("ed.toml"), edited).unwrap();
    crate::support::success(&run(directory.path(), &["gen", "authoring", "--offline"]));
    let candidate = source.replace("A line-oriented text editor", "Changed summary");
    assert_file(work.join(".cache/ed.candidate.spec"), &candidate);
    assert_file(work.join("ed.toml"), edited);
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &source);
    crate::support::success(&run(
        directory.path(),
        &["gen", "authoring", "--offline", "--output=."],
    ));
    assert_file(work.join("ed.spec"), &candidate);
    let completed: toml::Table =
        toml::from_str(&fs::read_to_string(work.join(".cache/ed.resolved.toml")).unwrap()).unwrap();
    assert_eq!(
        completed["edit"]["values"]["package"]["summary"].as_str(),
        Some("Changed summary")
    );
    fs::write(work.join("ed.toml"), "[package]\nsummary = ''\n").unwrap();
    let rejected = run(
        directory.path(),
        &["gen", "authoring", "--offline", "--output=rejected.spec"],
    );
    assert_eq!(rejected.status.code(), Some(1));
    assert!(!directory.path().join("rejected.spec").exists());
    assert_file(work.join(".cache/ed.candidate.spec"), &candidate);
    // Explicit auto publication updates only the bound checkout and advances its stage baseline.
    fs::write(work.join("ed.toml"), edited).unwrap();
    crate::support::success(&run(
        directory.path(),
        &["gen", "authoring", "--offline", "--apply"],
    ));
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &candidate);
    assert_file(directory.path().join("openruyi/SPECS/ed/ed.spec"), &source);
    crate::support::success(&run(directory.path(), &["gen", "authoring", "--offline"]));
    // A valid saved index is still not permission to target another package or source.
    fs::write(work.join("ed.toml"), edited).unwrap();
    let foreign = directory.path().join("foreign/ed.spec");
    fs::create_dir(foreign.parent().unwrap()).unwrap();
    fs::write(&foreign, &candidate).unwrap();
    let index_path = work.join(".state/index.toml");
    let mut index: toml::Value = toml::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
    index["drafts"][0]["source"] = foreign
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string()
        .into();
    fs::write(index_path, toml::to_string(&index).unwrap()).unwrap();
    let unrelated = run(directory.path(), &["gen", "authoring", "--offline"]);
    assert_eq!(unrelated.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unrelated.stderr).contains("not WORK's bound SPEC"));
    assert_file(work.join(".cache/ed.candidate.spec"), &candidate);
    assert_file(foreign, &candidate);
}

#[test]
fn indexed_generation_uses_local_admission_without_claiming_whole_spec_validity() {
    use super::support::{machine_report, success};
    let directory = tempfile::tempdir().unwrap();
    let source = SPEC.replace("GPL-3.0-or-later AND LGPL-2.1-or-later", "Invalid-License");
    let work = super::support::recipe_workspace(directory.path(), "authoring", "ed", &source);
    success(&run(
        directory.path(),
        &[
            "edit",
            "authoring",
            "--field",
            "package.summary",
            "--prepare",
            "work/authoring",
        ],
    ));
    fs::write(
        work.join("ed.toml"),
        "[package]\nsummary = 'Repair unrelated summary'\n",
    )
    .unwrap();
    let checked = run(
        directory.path(),
        &["gen", "authoring", "--offline", "--check", "--format=toml"],
    );
    success(&checked);
    let report = machine_report(&checked);
    assert_eq!(report["valid"].as_bool(), Some(false));
    assert_eq!(report["admissible"].as_bool(), Some(true));
    assert_eq!(report["success"].as_bool(), Some(true));
    let published = run(
        directory.path(),
        &["gen", "authoring", "--offline", "--output=review.spec"],
    );
    success(&published);
    assert_file(
        directory.path().join("review.spec"),
        &source.replace("A line-oriented text editor", "Repair unrelated summary"),
    );
}

#[test]
fn gen_auto_materializes_only_for_explicit_checked_spec_publication() {
    use super::support::success;
    use std::process::Command;
    let directory = workspace(MANIFEST);
    let recipes = directory.path().join("openruyi");
    fs::create_dir(&recipes).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&recipes)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
            .output()
            .unwrap();
        success(&output);
    };
    git(&["init", "--initial-branch=main", "--quiet"]);
    fs::write(recipes.join("README"), "recipe baseline\n").unwrap();
    git(&["add", "README"]);
    git(&["commit", "--quiet", "-m", "Fixture baseline"]);
    let work = directory.path().join("work/authoring");
    crate::support::success(&run(directory.path(), &["gen", "authoring", "--offline"]));
    assert!(!work.join("recipe").exists());
    crate::support::success(&run(
        directory.path(),
        &["gen", "authoring", "--offline", "--apply"],
    ));
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), SPEC);
    assert_file(work.join("ed.toml"), MANIFEST);
    assert!(!recipes.join("SPECS/ed/ed.spec").exists());
}

#[test]
fn completion_uses_binding_and_declared_build_contract_but_does_not_guess_package_facts() {
    let mut input: toml::Table = toml::from_str(MANIFEST).unwrap();
    input
        .get_mut("package")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .remove("name");
    input.remove("build-requires");
    let source = toml::to_string(&input).unwrap();
    let directory = workspace(&source);
    crate::support::success(&run(directory.path(), &["gen", "authoring", "--offline"]));
    let work = directory.path().join("work/authoring");
    let completed: toml::Table =
        toml::from_str(&fs::read_to_string(work.join(".cache/ed.resolved.toml")).unwrap()).unwrap();
    assert_eq!(
        completed["manifest"]["package"]["name"].as_str(),
        Some("ed")
    );
    assert_eq!(
        completed["manifest"]["build-requires"]["rpm"]
            .as_array()
            .unwrap(),
        &vec![
            "autoconf".into(),
            "automake".into(),
            "libtool".into(),
            "make".into()
        ]
    );
    assert_file(work.join("ed.toml"), &source);
    for field in ["version", "license"] {
        let mut missing = input.clone();
        missing
            .get_mut("package")
            .unwrap()
            .as_table_mut()
            .unwrap()
            .remove(field);
        let missing = toml::to_string(&missing).unwrap();
        let directory = workspace(&missing);
        let rejected = run(directory.path(), &["gen", "authoring", "--offline"]);
        assert_eq!(rejected.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains(&format!("missing field `{field}`"))
        );
        assert!(
            !directory
                .path()
                .join("work/authoring/.cache/ed.resolved.toml")
                .exists()
        );
        assert_file(directory.path().join("work/authoring/ed.toml"), &missing);
    }
}

#[test]
fn auto_rebase_projects_actual_declarations_without_backfilling_derived_missing_hash() {
    use super::support::success;
    let directory = tempfile::tempdir().unwrap();
    let work = super::support::recipe_workspace(directory.path(), "authoring", "ed", SPEC);
    success(&run(
        directory.path(),
        &[
            "edit",
            "authoring",
            "--field",
            "package.version",
            "--field",
            "sources.0",
            "--prepare",
            "work/authoring",
        ],
    ));
    fs::write(work.join("ed.toml"), "[package]\nversion = '2'\n[sources.0]\nurl = 'https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz'\n").unwrap();
    let generated = run(
        directory.path(),
        &["gen", "authoring", "--offline", "--apply"],
    );
    success(&generated);
    assert!(String::from_utf8_lossy(&generated.stderr).contains("declaration, not verification"));
    let completed: toml::Table =
        toml::from_str(&fs::read_to_string(work.join(".cache/ed.resolved.toml")).unwrap()).unwrap();
    assert!(
        completed["edit"]["values"]["sources"]["0"]
            .as_table()
            .unwrap()
            .get("sha256")
            .is_none()
    );
    let stage: toml::Table =
        toml::from_str(&fs::read_to_string(work.join("ed.toml")).unwrap()).unwrap();
    assert_eq!(
        stage["sources"]["0"]["sha256"].as_str(),
        Some("56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc")
    );
    let published = SPEC.replace("Version:        1.22.5", "Version:        2");
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &published);
    // The next editor iteration reads a shape-complete declaration baseline,
    // without treating the completed TOML or cached SPEC as a new authority.
    success(&run(
        directory.path(),
        &[
            "edit",
            "authoring",
            "--set",
            "package.summary=After gen",
            "--check",
            "--diff",
        ],
    ));
    assert_file(work.join("recipe/SPECS/ed/ed.spec"), &published);
    assert_file(directory.path().join("openruyi/SPECS/ed/ed.spec"), SPEC);
}

#[test]
fn late_completion_failure_reports_only_actual_spec_publication() {
    let directory = workspace(MANIFEST);
    let work = directory.path().join("work/authoring");
    fs::create_dir_all(work.join(".cache/ed.resolved.toml")).unwrap();
    let target = directory.path().join("review.spec");
    let written = run(
        directory.path(),
        &["gen", "authoring", "--offline", "--output=review.spec"],
    );
    assert_eq!(written.status.code(), Some(1));
    assert!(written.stdout.is_empty());
    let error = String::from_utf8_lossy(&written.stderr);
    assert!(
        error.contains("failed to write derived artifact"),
        "{error}"
    );
    assert!(error.contains("SPEC files already written:"), "{error}");
    assert!(
        error.contains(target.canonicalize().unwrap().to_str().unwrap()),
        "{error}"
    );
    assert!(error.contains("Publication was not rolled back"), "{error}");
    assert_file(&target, SPEC);
    assert_file(work.join("ed.toml"), MANIFEST);
    assert!(!work.join(".cache/ed.candidate.spec").exists());

    let unchanged = run(
        directory.path(),
        &["gen", "authoring", "--offline", "--output=review.spec"],
    );
    assert_eq!(unchanged.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&unchanged.stderr).contains("SPEC files already written:"));
    assert_file(&target, SPEC);
    fs::write(&target, "manual SPEC retained\n").unwrap();
    let skipped = run(
        directory.path(),
        &[
            "gen",
            "authoring",
            "--offline",
            "--output=review.spec",
            "--skip-existing",
        ],
    );
    assert_eq!(skipped.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&skipped.stderr).contains("SPEC files already written:"));
    assert_file(&target, "manual SPEC retained\n");
    for written_count in [1, 0] {
        let output = run(
            directory.path(),
            &[
                "gen",
                "authoring",
                "--offline",
                "--output=review.spec",
                "--force",
                "--format=toml",
            ],
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty(), "{output:?}");
        let report = super::support::machine_report(&output);
        assert_eq!(report["success"].as_bool(), Some(false));
        assert_eq!(report["written"].as_array().unwrap().len(), written_count);
        assert!(
            report["error"]["message"]
                .as_str()
                .unwrap()
                .contains("derived artifact")
        );
        assert_file(&target, SPEC);
    }
    fs::remove_dir(work.join(".cache/ed.resolved.toml")).unwrap();
    fs::create_dir(work.join(".cache/ed.candidate.spec")).unwrap();
    let output = run(
        directory.path(),
        &[
            "gen",
            "authoring",
            "--offline",
            "--output=review.spec",
            "--format=toml",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let report = super::support::machine_report(&output);
    assert!(report["written"].as_array().unwrap().is_empty());
    let artifacts = report["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(
        fs::canonicalize(artifacts[0].as_str().unwrap()).unwrap(),
        fs::canonicalize(work.join(".cache/ed.resolved.toml")).unwrap(),
    );
    assert_file(&target, SPEC);
    assert_file(work.join("ed.toml"), MANIFEST);
}

#[test]
fn generation_cannot_overwrite_its_input_or_alias() {
    use super::support::success;
    let directory = workspace(MANIFEST);
    let work = directory.path().join("work/authoring");
    for target in [work.join("ed.toml"), work.join(".lock")] {
        let before = fs::read(&target).unwrap();
        let output = run(
            directory.path(),
            &[
                "gen",
                "authoring",
                "--offline",
                "--force",
                &format!("--output={}", target.display()),
            ],
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(fs::read(target).unwrap(), before);
    }
    #[cfg(unix)]
    {
        let alias = directory.path().join("alias.spec");
        fs::hard_link(work.join("ed.toml"), &alias).unwrap();
        let output = run(
            directory.path(),
            &[
                "gen",
                "authoring",
                "--offline",
                "--force",
                &format!("--output={}", alias.display()),
            ],
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_file(alias, MANIFEST);
    }
    success(&run(
        directory.path(),
        &["gen", "authoring", "--offline", "--check"],
    ));
}
