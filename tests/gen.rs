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
        let directory = workspace(&source);
        let output = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&output);
        assert_eq!(
            output.stdout,
            SPEC.replace("# VCS: No VCS link available\n", expected)
                .as_bytes(),
            "{choice}"
        );
    }
}

#[test]
fn vcs_requires_one_valid_repository_choice_before_publication() {
    for choice in [
        "",
        "no-public-repository = false",
        "same-as-url = false\nno-public-repository = false",
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
fn stage_options_preserve_strings_and_order_without_changing_defaults() {
    let extra = r#"
[build.stages.check]
options = ["TESTS=smoke"]
[build.stages.install]
options = ['INSTALL="install -p"']
[build.stages.build]
options = ['CFLAGS="%{optflags} -fPIC"', "CC_FOR_BUILD=gcc"]
[build.stages.conf]
options = ["--enable-largefile", "--enable-nls", "--disable-rpath"]
[build.stages.prep]
options = ["-p0"]
"#;
    let directory = workspace(&format!("{MANIFEST}{extra}"));
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    let expected = SPEC.replace(
        "BuildRequires:  autoconf\n",
        concat!(
            "BuildOption(prep):  -p0\n",
            "BuildOption(conf):  --enable-largefile\n",
            "BuildOption(conf):  --enable-nls\n",
            "BuildOption(conf):  --disable-rpath\n",
            "BuildOption(build):  CFLAGS=\"%{optflags} -fPIC\"\n",
            "BuildOption(build):  CC_FOR_BUILD=gcc\n",
            "BuildOption(install):  INSTALL=\"install -p\"\n",
            "BuildOption(check):  TESTS=smoke\n\n",
            "BuildRequires:  autoconf\n",
        ),
    );
    assert_eq!(output.stdout, expected.as_bytes());

    for extra in [
        "\n[build.stages.conf]\n",
        "\n[build.stages.conf]\noptions = []\n",
    ] {
        fs::write(
            directory.path().join("ed.toml"),
            format!("{MANIFEST}{extra}"),
        )
        .unwrap();
        let output = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&output);
        assert_eq!(output.stdout, SPEC.as_bytes());
    }
}

#[test]
fn invalid_stage_options_are_rejected_before_publication() {
    for extra in [
        "\n[build.stages.conf]\noptions = [\"\"]\n",
        "\n[build.stages.conf]\noptions = [\" --enable-nls\"]\n",
        "\n[build.stages.conf]\noptions = [\"--enable-nls\\nVersion: 2\"]\n",
        "\n[build.stages.conf]\noptions = [\"--enable-nls\\\\\"]\n",
    ] {
        rejected(&format!("{MANIFEST}{extra}"), "build.stages.conf.options");
    }
    for (extra, error) in [
        (
            "\n[build.stages.configure]\noptions = []\n",
            "unknown variant `configure`",
        ),
        (
            "\n[build.stages.conf]\noption = []\n",
            "unknown field `option`",
        ),
        (
            "\n[build.stages.conf]\noptions = \"--enable-nls\"\n",
            "expected a sequence",
        ),
    ] {
        rejected(&format!("{MANIFEST}{extra}"), error);
    }
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
        "https://example.org/a%%20b.tar.lz",
    ] {
        let directory = workspace(&MANIFEST.replace(SOURCE, url));
        let output = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&output);
        assert_eq!(output.stdout, SPEC.replace(SOURCE, url).as_bytes());
    }
    for url in [
        "https:/example.org/ed.tar.lz",
        // The pinned rpm-spec parser represents these as unsupported macro tokens.
        "https://example.org/a%20b.tar.lz",
        "https://example.org/%AF.tar.lz",
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

#[test]
fn stage_scripts_preserve_bodies_and_keep_default_actions() {
    let extra = r#"
[build.stages.install]
append = '''
# Remove an upstream-created directory index.
rm -f %{buildroot}%{_infodir}/dir
'''
[build.stages.conf]
prepend = 'autoreconf -fiv'
append = "echo configured\n"
options = ["--enable-nls"]
"#;
    let directory = workspace(&format!("{MANIFEST}{extra}"));
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    let expected = SPEC
        .replace(
            "BuildRequires:  autoconf\n",
            "BuildOption(conf):  --enable-nls\n\nBuildRequires:  autoconf\n",
        )
        .replace(
            "%files\n",
            concat!(
                "%conf -p\nautoreconf -fiv\n\n",
                "%conf -a\necho configured\n\n",
                "%install -a\n# Remove an upstream-created directory index.\n",
                "rm -f %{buildroot}%{_infodir}/dir\n\n%files\n",
            ),
        );
    assert_eq!(output.stdout, expected.as_bytes());

    // Blank lines and tabs inside a heredoc are script data, not layout to trim.
    let script = "cat <<'END' > generated.txt\n\tindented\n\nEND\n\n";
    for stage in ["prep", "conf", "build", "install", "check"] {
        for (field, flag) in [("prepend", "p"), ("append", "a")] {
            let extra = format!("\n[build.stages.{stage}]\n{field} = '''{script}'''\n");
            fs::write(
                directory.path().join("ed.toml"),
                format!("{MANIFEST}{extra}"),
            )
            .unwrap();
            let output = run(directory.path(), &["gen", "ed", "--stdout"]);
            success(&output);
            let expected =
                SPEC.replace("%files\n", &format!("%{stage} -{flag}\n{script}\n%files\n"));
            assert_eq!(output.stdout, expected.as_bytes());
        }
    }
    fs::write(
        directory.path().join("ed.toml"),
        format!("{MANIFEST}\n[build.stages.conf]\nprepend = ''\nappend = ''\n"),
    )
    .unwrap();
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    assert_eq!(output.stdout, SPEC.as_bytes());
}

#[test]
fn stage_scripts_reject_literal_section_headers_and_invalid_text() {
    for field in ["append", "replace"] {
        let path = format!("build.stages.conf.{field}");
        for (script, message) in [
            (r"echo bad\u0000", path.as_str()),
            (r"echo bad\r\n", path.as_str()),
            (r"echo one\n%files\n/unexpected", path.as_str()),
            (r"echo one\n%install\necho two", path.as_str()),
            (r"%if 1\necho unfinished", "parser diagnostics"),
        ] {
            let extra = format!("\n[build.stages.conf]\n{field} = \"{script}\"\n");
            rejected(&format!("{MANIFEST}{extra}"), message);
        }
    }
}

#[test]
fn stage_replacement_preserves_empty_actions_and_hooks() {
    for stage in ["prep", "conf", "build", "install", "check"] {
        for script in [
            "",
            "# No action needed.",
            "cat <<'END'\n\tcontent\n\nEND\n\n",
        ] {
            let extra = format!(
                "\n[build.stages.{stage}]\noptions = []\nprepend = 'echo before'\n\
                 replace = '''{script}'''\nappend = 'echo after'\n"
            );
            let directory = workspace(&format!("{MANIFEST}{extra}"));
            let output = run(directory.path(), &["gen", "ed", "--stdout"]);
            success(&output);
            let body = match script {
                "" => String::new(),
                s if s.ends_with('\n') => s.into(),
                s => format!("{s}\n"),
            };
            let expected = SPEC.replace(
                "%files\n",
                &format!(
                    "%{stage} -p\necho before\n\n%{stage}\n{body}\n\
                     %{stage} -a\necho after\n\n%files\n"
                ),
            );
            assert_eq!(output.stdout, expected.as_bytes(), "{stage}: {script:?}");
        }
    }
}

#[test]
fn stage_options_and_replacement_are_mutually_exclusive_per_stage() {
    for script in ["", "echo custom"] {
        rejected(
            &format!(
                "{MANIFEST}\n[build.stages.conf]\noptions = ['--enable-nls']\nreplace = '{script}'\n"
            ),
            "build.stages.conf: options cannot be combined with replace",
        );
    }
    let directory = workspace(&format!(
        "{MANIFEST}\n[build.stages.conf]\nreplace = ''\n\
         [build.stages.build]\noptions = ['CC_FOR_BUILD=gcc']\n"
    ));
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    success(&output);
    let expected = SPEC.replace("%files\n", "%conf\n\n%files\n").replace(
        "BuildRequires:  autoconf\n",
        "BuildOption(build):  CC_FOR_BUILD=gcc\n\nBuildRequires:  autoconf\n",
    );
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn omitted_build_system_adds_no_stages_or_requirements() {
    let source = MANIFEST
        .replace("[build]\nsystem = \"autotools\"\n", "")
        .replace(
            "[\"autoconf\", \"automake\", \"libtool\", \"make\", \"lzip\"]",
            "[]",
        );
    let expected = SPEC.replace("BuildSystem:    autotools\n", "").replace(
        concat!(
            "BuildRequires:  autoconf\nBuildRequires:  automake\n",
            "BuildRequires:  libtool\nBuildRequires:  make\nBuildRequires:  lzip\n\n",
        ),
        "",
    );
    for extra in [
        "",
        "\n[build]\n",
        "\n[build.stages.conf]\noptions = []\nprepend = ''\nappend = ''\n",
    ] {
        let directory = workspace(&format!("{source}{extra}"));
        let output = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&output);
        assert_eq!(output.stdout, expected.as_bytes(), "{extra}");
    }
}

#[test]
fn explicit_stages_work_without_build_system_defaults() {
    let source = MANIFEST
        .replace("system = \"autotools\"\n", "")
        .replace("\"autoconf\", \"automake\", \"libtool\", ", "");
    let extra = r#"
[build.stages.prep]
replace = '%autosetup -p1'
[build.stages.install]
prepend = 'echo before-install'
replace = '%make_install'
append = 'echo after-install'
"#;
    let expected = SPEC.replace("BuildSystem:    autotools\n", "").replace(
        "BuildRequires:  autoconf\nBuildRequires:  automake\nBuildRequires:  libtool\n",
        "",
    );
    // Hooks also work without a main section; no implicit action is inserted.
    for main in ["%make_install", ""] {
        let extra = if main.is_empty() {
            extra.replace("replace = '%make_install'\n", "")
        } else {
            extra.into()
        };
        let directory = workspace(&format!("{source}{extra}"));
        let output = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&output);
        let main = if main.is_empty() {
            String::new()
        } else {
            format!("%install\n{main}\n\n")
        };
        let expected = expected.replace(
            "%files\n",
            &format!(
                "%prep\n%autosetup -p1\n\n%install -p\necho before-install\n\n\
                 {main}%install -a\necho after-install\n\n%files\n"
            ),
        );
        assert_eq!(output.stdout, expected.as_bytes());
    }
}

#[test]
fn explicit_build_keeps_validation_and_rejects_options_without_defaults() {
    let source = MANIFEST.replace("system = \"autotools\"\n", "");
    rejected(
        &format!("{source}\n[build.stages.conf]\noptions = ['--enable-nls']\n"),
        "build.stages.conf.options: requires build.system",
    );
    for system in ["", "plain"] {
        rejected(
            &MANIFEST.replace("system = \"autotools\"", &format!("system = '{system}'")),
            "build.system: unsupported build system",
        );
    }
    rejected(
        &source.replace("\"lzip\"", "\"lzip >=\""),
        "parser diagnostics",
    );
    rejected(
        &source.replace("GPL-3.0-or-later AND LGPL-2.1-or-later", "MIT AND"),
        "package.license",
    );
}

#[test]
fn generated_vcs_and_scripts_offer_selected_editing_when_full_views_are_unsupported() {
    for manifest in [
        MANIFEST.replace(
            "no-public-repository = true",
            "git = \"https://example.org/ed.git\"",
        ),
        format!(
            "{}\n[build.stages.build]\nreplace = '%make_build'\n",
            MANIFEST.replace("system = \"autotools\"", "")
        ),
    ] {
        let directory = workspace(&manifest);
        let generated = run(directory.path(), &["gen", "ed", "--stdout"]);
        success(&generated);
        fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
        let full = run(directory.path(), &["edit", "ed.spec", "--view"]);
        assert_eq!(full.status.code(), Some(1));
        assert!(full.stdout.is_empty());
        let error = String::from_utf8_lossy(&full.stderr);
        assert!(error.contains("--field package.version --view"), "{error}");
        success(&run(
            directory.path(),
            &["edit", "ed.spec", "--field", "package.version", "--view"],
        ));
        let diff = run(
            directory.path(),
            &[
                "edit",
                "ed.spec",
                "--set",
                "package.version=1.22.6",
                "--diff",
            ],
        );
        success(&diff);
        assert!(String::from_utf8_lossy(&diff.stdout).contains("+Version:        1.22.6"));
        assert_eq!(
            fs::read(directory.path().join("ed.spec")).unwrap(),
            generated.stdout
        );
    }
}

#[test]
fn source_without_a_digest_renders_a_bare_remote_asset_and_warns() {
    // openRuyi accepts a bare #!RemoteAsset for a source with no recorded digest
    // (about 40% of cmake/meson packages use it). Omitting the sha256 key opts in.
    let manifest = MANIFEST.replace(
        "sha256 = \"56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc\"\n",
        "",
    );
    let directory = workspace(&manifest);
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let spec = String::from_utf8_lossy(&output.stdout);
    // Bare marker on its own line, not the sha256 form.
    assert!(spec.contains("#!RemoteAsset\nSource0:"), "{spec}");
    assert!(!spec.contains("#!RemoteAsset:  sha256:"), "{spec}");
    // Weaker provenance is surfaced on stderr, not silently emitted.
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no sha256"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn an_empty_sha256_is_rejected_rather_than_treated_as_bare() {
    // A blank scaffold field must not silently opt into the bare form; only an
    // absent key does. This keeps the missing-field report honest.
    let manifest = MANIFEST.replace(
        "sha256 = \"56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc\"",
        "sha256 = \"\"",
    );
    rejected(&manifest, "sources.0.sha256");
}
