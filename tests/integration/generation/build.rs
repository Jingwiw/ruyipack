// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Build-system defaults, explicit stages, hooks, and replacement.

use super::{MANIFEST, SPEC, quiet_success, rejected, renders, run, workspace};
use std::fs;

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
    let output = run(directory.path(), &["gen", "authoring", "--stdout"]);
    quiet_success(&output);
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
            directory.path().join("work/authoring/ed.toml"),
            format!("{MANIFEST}{extra}"),
        )
        .unwrap();
        let output = run(directory.path(), &["gen", "authoring", "--stdout"]);
        quiet_success(&output);
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
    renders(&format!("{MANIFEST}{extra}"), &expected);

    // Blank lines and tabs inside a heredoc are script data, not layout to trim.
    let script = "cat <<'END' > generated.txt\n\tindented\n\nEND\n\n";
    for stage in ["prep", "conf", "build", "install", "check"] {
        for (field, flag) in [("prepend", "p"), ("append", "a")] {
            let extra = format!("\n[build.stages.{stage}]\n{field} = '''{script}'''\n");
            let expected =
                SPEC.replace("%files\n", &format!("%{stage} -{flag}\n{script}\n%files\n"));
            renders(&format!("{MANIFEST}{extra}"), &expected);
        }
    }
    renders(
        &format!("{MANIFEST}\n[build.stages.conf]\nprepend = ''\nappend = ''\n"),
        SPEC,
    );
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
            renders(&format!("{MANIFEST}{extra}"), &expected);
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
    let source = format!(
        "{MANIFEST}\n[build.stages.conf]\nreplace = ''\n\
         [build.stages.build]\noptions = ['CC_FOR_BUILD=gcc']\n"
    );
    let expected = SPEC.replace("%files\n", "%conf\n\n%files\n").replace(
        "BuildRequires:  autoconf\n",
        "BuildOption(build):  CC_FOR_BUILD=gcc\n\nBuildRequires:  autoconf\n",
    );
    renders(&source, &expected);
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
        renders(&format!("{source}{extra}"), &expected);
    }
}

#[test]
fn explicit_stages_work_without_build_system_defaults() {
    let source = MANIFEST
        .replace("system = \"autotools\"\n", "")
        .replace("\"autoconf\", \"automake\", \"libtool\", ", "");
    let extra = r"
[build.stages.prep]
replace = '%autosetup -p1'
[build.stages.install]
prepend = 'echo before-install'
replace = '%make_install'
append = 'echo after-install'
";
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
        renders(&format!("{source}{extra}"), &expected);
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
        let generated = run(directory.path(), &["gen", "authoring", "--stdout"]);
        quiet_success(&generated);
        fs::write(directory.path().join("ed.spec"), &generated.stdout).unwrap();
        let full = run(
            directory.path(),
            &["inspect", "--spec=ed.spec", "--all", "--editable"],
        );
        assert_eq!(full.status.code(), Some(1));
        assert!(full.stdout.is_empty());
        let error = String::from_utf8_lossy(&full.stderr);
        assert!(
            error.contains("unsupported") || error.contains("section:"),
            "{error}"
        );
        quiet_success(&run(
            directory.path(),
            &[
                "inspect",
                "--spec=ed.spec",
                "--field",
                "package.version",
                "--editable",
            ],
        ));
        let diff = run(
            directory.path(),
            &[
                "edit",
                "--spec=ed.spec",
                "--set",
                "package.version=1.22.6",
                "--diff",
            ],
        );
        assert!(diff.status.success(), "{diff:?}");
        let stderr = String::from_utf8_lossy(&diff.stderr);
        assert!(stderr.lines().count() >= 1);
        assert!(stderr.contains("unverified source-authenticity, patch-applicability, native-build; triggers=package.version"));
        assert!(String::from_utf8_lossy(&diff.stdout).contains("+Version:        1.22.6"));
        assert_eq!(
            fs::read(directory.path().join("ed.spec")).unwrap(),
            generated.stdout
        );
    }
}
