// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Regression checks for syntactically valid corruption of generated facts.

use super::run;
use crate::spec::ParsedSpec;
use crate::{
    profile,
    render::{manifest, spec},
};

const MANIFEST: &str = include_str!("../../../examples/ed/ed.toml");

#[test]
fn rejects_changed_facts_even_when_the_spec_still_parses() {
    let recipe = manifest::parse(MANIFEST).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    for (before, after, field) in [
        ("Name:           ed", "Name:           another", "Name"),
        ("1.22.5", "1.22.6", "Version"),
        ("%autorelease", "2%{?dist}", "Release"),
        ("A line-oriented text editor", "Another editor", "Summary"),
        ("Summary:", "Summary(fr):", "Summary"),
        ("GPL-3.0-or-later AND LGPL-2.1-or-later", "MIT", "License"),
        (
            "https://www.gnu.org/software/ed/",
            "https://example.org/ed/",
            "URL",
        ),
        ("Source0:", "Source1:", "Source"),
        (".tar.lz", ".tar.gz", "Source"),
        ("sha256:56e107", "sha256:66e107", "sources.0.sha256"),
        (
            "BuildSystem:    autotools",
            "BuildSystem:    cmake",
            "BuildSystem",
        ),
        (
            "BuildRequires:  lzip",
            "BuildRequires:  gzip",
            "BuildRequires",
        ),
        (
            "BuildRequires:  lzip",
            "BuildRequires:  lzip >= 2",
            "BuildRequires",
        ),
        (
            "BuildRequires:  lzip",
            "BuildRequires(pre): lzip",
            "BuildRequires",
        ),
        (
            "GNU ed is a line-oriented",
            "GNU ed was a line-oriented",
            "package.description",
        ),
        ("%license COPYING", "%doc COPYING", "package.files"),
        (
            "AUTHORS ChangeLog NEWS README",
            "AUTHORS ChangeLog README",
            "package.files",
        ),
        (
            "%{_bindir}/r%{name}",
            "%{_sbindir}/r%{name}",
            "package.files",
        ),
        ("%{?ext_info}", "%{ext_info}", "package.files"),
        ("%autochangelog", "%{?autochangelog}", "changelog"),
        ("(C) 2025", "(C) 2024", "SPEC metadata"),
        ("Zheng Junjie", "Another Contributor", "SPEC metadata"),
        (
            "Identifier: MulanPSL-2.0",
            "Identifier: Apache-2.0",
            "SPEC metadata",
        ),
        (
            "# VCS: No VCS link available",
            "# VCS: https://example.org/ed.git",
            "SPEC metadata",
        ),
    ] {
        let changed = original.replacen(before, after, 1);
        assert_ne!(changed, original, "mutation did not apply: {before}");
        let parsed = ParsedSpec::parse(&changed);
        assert!(
            parsed.parsed.diagnostics.is_empty(),
            "not a clean parser result: {before}"
        );
        let error = run(&parsed, &recipe, &profile).unwrap_err().to_string();
        assert!(error.contains(field), "{before}: {error}");
    }
}

#[test]
fn rejects_changed_vcs_declarations() {
    for choice in [
        "git = \"https://example.org/project.git\"",
        "same-as-url = true",
        "no-public-repository = true",
    ] {
        let input = MANIFEST.replace("no-public-repository = true", choice);
        let recipe = manifest::parse(&input).unwrap();
        let profile = profile::load().unwrap();
        let original = spec::render(&recipe, &profile);
        assert!(run(&ParsedSpec::parse(&original), &recipe, &profile).is_ok());
        for declaration in [
            "VCS: git:https://example.org/another.git\n",
            "# VCS: No VCS link available\n",
        ] {
            let changed = original.replace("BuildSystem:", &format!("{declaration}BuildSystem:"));
            let parsed = ParsedSpec::parse(&changed);
            assert!(parsed.parsed.diagnostics.is_empty());
            assert!(
                run(&parsed, &recipe, &profile).is_err(),
                "{choice}: {declaration}"
            );
        }
        if choice.starts_with("git =") {
            for replacement in ["", "VCS: git:https://example.org/another.git\n"] {
                let changed = original.replace(
                    "VCS:            git:https://example.org/project.git\n",
                    replacement,
                );
                let parsed = ParsedSpec::parse(&changed);
                assert!(parsed.parsed.diagnostics.is_empty());
                assert!(run(&parsed, &recipe, &profile).is_err());
            }
        }
    }
}

#[test]
fn rejects_missing_duplicate_and_unexpected_units() {
    let recipe = manifest::parse(MANIFEST).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    for changed in [
        original.replace("Version:        1.22.5\n", ""),
        original.replace("Version:        1.22.5", "Version: 1.22.5\nVersion: 1.22.5"),
        original.replace("Version:        1.22.5", "Version: 1.22.5\nEpoch: 1"),
        original.replace("%files\n", "%files -f extra.files\n"),
        original.replace("%{_bindir}/r%{name}\n", ""),
        original.replace(
            "%{_bindir}/r%{name}\n",
            "%{_bindir}/r%{name}\n%{_bindir}/r%{name}\n",
        ),
        original.replace("\n%changelog\n%autochangelog\n", ""),
    ] {
        let parsed = ParsedSpec::parse(&changed);
        assert!(parsed.parsed.diagnostics.is_empty());
        assert!(run(&parsed, &recipe, &profile).is_err());
    }
}

#[test]
fn keeps_each_checksum_bound_to_its_source() {
    let source = format!(
        "{MANIFEST}\n[sources.2]\nurl = \"https://example.org/manual.tar.gz\"\nsha256 = \"{}\"\n",
        "a".repeat(64)
    );
    let recipe = manifest::parse(&source).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    let source_lines: Vec<_> = original
        .lines()
        .filter(|line| line.starts_with("Source"))
        .collect();
    let mut lines: Vec<_> = original.lines().collect();
    let positions: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| line.starts_with("Source").then_some(i))
        .collect();
    lines[positions[0]] = source_lines[1];
    lines[positions[1]] = source_lines[0];
    // All tags and all comments remain present; only their association is wrong.
    let changed = lines.join("\n") + "\n";
    let parsed = ParsedSpec::parse(&changed);
    assert!(parsed.parsed.diagnostics.is_empty());
    assert!(
        run(&parsed, &recipe, &profile)
            .unwrap_err()
            .to_string()
            .contains("sources.2.sha256")
    );
}

#[test]
fn ignores_layout_but_preserves_prose_and_macro_structure() {
    let mut recipe = manifest::parse(MANIFEST).unwrap();
    recipe.package.body.description =
        "An editor with %{name} and 100%% text.\n  Indented text  \n \n".into();
    recipe
        .build_requires
        .rpm
        .push("pkgconfig(example) >= 1.2".into());
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    let changed = original
        .replace("Name:           ", "Name:\t")
        .replace("%files\n", "%files\n\n");
    assert!(run(&ParsedSpec::parse(&changed), &recipe, &profile).is_ok());
    let changed = original.replace("  Indented text  ", "Indented text");
    assert!(run(&ParsedSpec::parse(&changed), &recipe, &profile).is_err());
}

#[test]
fn preserves_remote_asset_marker_bytes() {
    let recipe = manifest::parse(MANIFEST).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    assert!(run(&ParsedSpec::parse(&original), &recipe, &profile).is_ok());

    let changed = original.replacen("#!RemoteAsset:", "# !RemoteAsset:", 1);
    assert_ne!(changed, original);
    let parsed = ParsedSpec::parse(&changed);
    assert!(parsed.parsed.diagnostics.is_empty());
    assert!(
        run(&parsed, &recipe, &profile)
            .unwrap_err()
            .to_string()
            .contains("sources.0.sha256")
    );
}

#[test]
fn stage_options_match_their_stage_value_and_order() {
    let input = format!(
        "{MANIFEST}\n[build.stages.conf]\noptions = [\"--enable-nls\", \"--disable-rpath\"]\n\
         [build.stages.build]\noptions = [\"CC_FOR_BUILD=gcc\"]\n"
    );
    let recipe = manifest::parse(&input).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    assert!(run(&ParsedSpec::parse(&original), &recipe, &profile).is_ok());
    for (before, after) in [
        ("BuildOption(conf):  --enable-nls\n", ""),
        ("--enable-nls", "--disable-nls"),
        ("BuildOption(conf):", "BuildOption(install):"),
        ("BuildOption(conf):", "BuildOption:"),
        (
            "BuildOption(conf):  --enable-nls\nBuildOption(conf):  --disable-rpath\n",
            "BuildOption(conf):  --disable-rpath\nBuildOption(conf):  --enable-nls\n",
        ),
        (
            "BuildOption(conf):  --enable-nls\n",
            "BuildOption(conf):  --enable-nls\nBuildOption(conf):  --enable-nls\n",
        ),
    ] {
        let changed = original.replacen(before, after, 1);
        assert_ne!(changed, original);
        let parsed = ParsedSpec::parse(&changed);
        assert!(parsed.parsed.diagnostics.is_empty(), "{before}");
        assert!(run(&parsed, &recipe, &profile).is_err(), "{before}");
    }
}

#[test]
fn stage_scripts_match_kind_placement_and_exact_body() {
    let input = format!(
        "{MANIFEST}\n[build.stages.conf]\nprepend = 'autoreconf -fiv'\n\
         append = '''echo configured\n'''\n\
         [build.stages.install]\nappend = '''cat <<'END' > generated\n\tcontent\n\nEND\n\n'''\n"
    );
    let recipe = manifest::parse(&input).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    assert!(run(&ParsedSpec::parse(&original), &recipe, &profile).is_ok());
    for (before, after) in [
        ("%conf -p", "%build -p"),
        ("%conf -p", "%conf -a"),
        ("%conf -p", "%conf"),
        ("autoreconf -fiv", "autoreconf -fi"),
        ("\tcontent", "content"),
        ("content\n\nEND", "content\nEND"),
        ("END\n\n\n", "END\n\n"),
        ("%conf -p\nautoreconf -fiv\n\n", ""),
        (
            "%conf -p\nautoreconf -fiv\n\n",
            "%conf -p\nautoreconf -fiv\n\n%conf -p\nautoreconf -fiv\n\n",
        ),
        (
            "%conf -p\nautoreconf -fiv\n\n%conf -a\necho configured\n\n",
            "%conf -a\necho configured\n\n%conf -p\nautoreconf -fiv\n\n",
        ),
    ] {
        let changed = original.replacen(before, after, 1);
        assert_ne!(changed, original);
        let parsed = ParsedSpec::parse(&changed);
        assert!(parsed.parsed.diagnostics.is_empty(), "{before}");
        assert!(run(&parsed, &recipe, &profile).is_err(), "{before}");
    }
}

#[test]
fn stage_replacement_checks_explicit_main_sections() {
    let input = format!(
        "{MANIFEST}\n[build.stages.conf]\nreplace = ''\n\
         [build.stages.check]\nreplace = '# Tests require unavailable hardware.'\n"
    );
    let recipe = manifest::parse(&input).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    assert!(run(&ParsedSpec::parse(&original), &recipe, &profile).is_ok());
    for (before, after) in [
        ("%conf\n\n", ""),
        ("%conf\n", "%conf -p\n"),
        ("%conf\n", "%conf -a\n"),
        ("%conf\n", "%build\n"),
        ("%conf\n", "%conf\necho unexpected\n"),
        ("%conf\n\n", "%conf\n\n%conf\n\n"),
        ("# Tests require unavailable hardware.", "# No tests."),
    ] {
        let changed = original.replacen(before, after, 1);
        assert_ne!(changed, original);
        let parsed = ParsedSpec::parse(&changed);
        assert!(parsed.parsed.diagnostics.is_empty(), "{before}");
        assert!(run(&parsed, &recipe, &profile).is_err(), "{before}");
    }
}

#[test]
fn build_system_presence_matches_the_manifest() {
    let mut recipe = manifest::parse(MANIFEST).unwrap();
    let profile = profile::load().unwrap();
    let declarative = spec::render(&recipe, &profile);
    recipe.build.system = None;
    let explicit = spec::render(&recipe, &profile);
    assert!(run(&ParsedSpec::parse(&explicit), &recipe, &profile).is_ok());
    assert!(run(&ParsedSpec::parse(&declarative), &recipe, &profile).is_err());
    recipe.build.system = Some("autotools".into());
    assert!(run(&ParsedSpec::parse(&explicit), &recipe, &profile).is_err());
}

const SUBPACKAGES: &str = r#"
[subpackages.devel]
summary = "Development files"
description = "Headers for ed development."
requires = ["%{name} = %{version}-%{release}", "pkgconfig(example) >= 1"]
provides = ["ed-devel-api = %{version}"]
[subpackages.devel.files]
license = ["COPYING.devel"]
doc = ["README.devel"]
entries = ["%{_includedir}/ed.h"]

[subpackages.editor-tools]
full-name = true
summary = "Editor tools"
description = "Standalone editor utilities."
requires = ["coreutils"]
provides = ["editor-helper = 1"]
[subpackages.editor-tools.files]
entries = ["%{_bindir}/editor-helper"]

[subpackages.meta]
summary = "Editor metapackage"
description = "Install the editor family."
"#;

#[test]
fn subpackage_facts_are_verified_independently_of_the_main_package() {
    let recipe = manifest::parse(&format!("{MANIFEST}{SUBPACKAGES}")).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    let parsed = ParsedSpec::parse(&original);
    assert!(parsed.parsed.diagnostics.is_empty());
    assert!(run(&parsed, &recipe, &profile).is_ok());
    for (before, after) in [
        ("%package        devel", "%package        headers"),
        ("%package        devel", "%package        -n devel"),
        (
            "%package        -n editor-tools",
            "%package        editor-tools",
        ),
        ("%description    devel", "%description    headers"),
        ("%description    devel", "%description    -n devel"),
        (
            "%description    -n editor-tools",
            "%description    editor-tools",
        ),
        ("%files devel", "%files headers"),
        ("%files devel", "%files -n devel"),
        ("%files -n editor-tools", "%files editor-tools"),
        (
            "%files -n editor-tools",
            "%files -n unexpected -n editor-tools",
        ),
        (
            "%files -n editor-tools",
            "%files unexpected -n editor-tools",
        ),
        ("%files devel", "%files -f extra.files devel"),
        (
            "Summary:        Development files",
            "Summary:        Other headers",
        ),
        (
            "Summary:        Development files",
            "Summary(fr):    Development files",
        ),
        (
            "Requires:       %{name} = %{version}-%{release}",
            "Requires:       %{name} >= %{version}-%{release}",
        ),
        (
            "Requires:       pkgconfig(example) >= 1",
            "Requires(pre):  pkgconfig(example) >= 1",
        ),
        (
            "Provides:       ed-devel-api = %{version}",
            "Provides:       ed-devel-api = 2",
        ),
        ("Headers for ed development.", "Headers for another editor."),
        ("%license COPYING.devel", "%doc COPYING.devel"),
        ("%doc README.devel", "%doc NEWS.devel"),
        ("%{_includedir}/ed.h", "%{_includedir}/red.h"),
        ("%files meta\n", "%files meta\n/usr/share/unexpected\n"),
    ] {
        let changed = original.replacen(before, after, 1);
        assert_ne!(changed, original, "mutation did not apply: {before}");
        let parsed = ParsedSpec::parse(&changed);
        assert!(
            parsed.parsed.diagnostics.is_empty(),
            "not a clean parser result: {before}"
        );
        assert!(
            run(&parsed, &recipe, &profile).is_err(),
            "accepted: {before}"
        );
    }
}

#[test]
fn subpackages_reject_missing_duplicate_and_unexpected_units() {
    let recipe = manifest::parse(&format!("{MANIFEST}{SUBPACKAGES}")).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    assert!(run(&ParsedSpec::parse(&original), &recipe, &profile).is_ok());
    for (before, after) in [
        ("Summary:        Development files\n", ""),
        (
            "Summary:        Development files\n",
            "Summary:        Development files\nSummary: Development files\n",
        ),
        (
            "Summary:        Development files\n",
            "Summary:        Development files\nLicense: MIT\n",
        ),
        ("Requires:       coreutils\n", ""),
        (
            "Requires:       coreutils\n",
            "Requires:       coreutils\nRequires: coreutils\n",
        ),
        ("Provides:       editor-helper = 1\n", ""),
        (
            "Provides:       editor-helper = 1\n",
            "Provides:       editor-helper = 1\nProvides: editor-helper = 1\n",
        ),
        ("%description    devel\nHeaders for ed development.\n\n", ""),
        (
            "%description    devel\nHeaders for ed development.\n\n",
            "%description devel\nHeaders for ed development.\n\n%description devel\nHeaders for ed development.\n\n",
        ),
        ("%files -n editor-tools\n%{_bindir}/editor-helper\n\n", ""),
        (
            "%files -n editor-tools\n%{_bindir}/editor-helper\n\n",
            "%files -n editor-tools\n%{_bindir}/editor-helper\n\n%files -n editor-tools\n%{_bindir}/editor-helper\n\n",
        ),
        ("%{_includedir}/ed.h\n", ""),
        (
            "%{_includedir}/ed.h\n",
            "%{_includedir}/ed.h\n%{_includedir}/ed.h\n",
        ),
        (
            "%package        meta\nSummary:        Editor metapackage\n\n",
            "",
        ),
        (
            "%package        meta\nSummary:        Editor metapackage\n\n",
            "%package meta\nSummary: Editor metapackage\n\n%package meta\nSummary: Editor metapackage\n\n",
        ),
    ] {
        let changed = original.replacen(before, after, 1);
        assert_ne!(changed, original, "mutation did not apply: {before}");
        let parsed = ParsedSpec::parse(&changed);
        assert!(
            parsed.parsed.diagnostics.is_empty(),
            "not a clean parser result: {before}"
        );
        assert!(
            run(&parsed, &recipe, &profile).is_err(),
            "accepted: {before}"
        );
    }
    let extra_tail = format!("{original}\n%files unexpected\n");
    let parsed = ParsedSpec::parse(&extra_tail);
    assert!(parsed.parsed.diagnostics.is_empty());
    let error = run(&parsed, &recipe, &profile).unwrap_err().to_string();
    assert!(error.contains("unexpected sections"), "{error}");
}

#[test]
fn subpackage_facts_cannot_be_moved_between_package_scopes() {
    let recipe = manifest::parse(&format!("{MANIFEST}{SUBPACKAGES}")).unwrap();
    let profile = profile::load().unwrap();
    let original = spec::render(&recipe, &profile);
    assert!(run(&ParsedSpec::parse(&original), &recipe, &profile).is_ok());
    for (left, right) in [
        ("Development files", "Editor tools"),
        (
            "Headers for ed development.",
            "Standalone editor utilities.",
        ),
        (
            "Requires:       %{name} = %{version}-%{release}",
            "Requires:       coreutils",
        ),
        (
            "Provides:       ed-devel-api = %{version}",
            "Provides:       editor-helper = 1",
        ),
        ("%{_includedir}/ed.h", "%{_bindir}/editor-helper"),
    ] {
        // Preserve the complete set of facts but attach each pair to the wrong
        // package. Counting tags globally is not an adequate round-trip check.
        let changed = original
            .replace(left, "SWAPPED_TEST_FACT")
            .replace(right, left)
            .replace("SWAPPED_TEST_FACT", right);
        assert_ne!(changed, original);
        let parsed = ParsedSpec::parse(&changed);
        assert!(parsed.parsed.diagnostics.is_empty(), "{left}");
        assert!(run(&parsed, &recipe, &profile).is_err(), "accepted: {left}");
    }
}
