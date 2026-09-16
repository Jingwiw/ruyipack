// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Regression checks for syntactically valid corruption of generated facts.

use super::run;
use crate::render::{manifest, profile, spec};
use crate::spec::ParsedSpec;

const MANIFEST: &str = include_str!("../../../examples/ed/ed.toml");

#[test]
fn rejects_changed_facts_even_when_the_spec_still_parses() {
    let recipe = manifest::parse(MANIFEST).unwrap();
    let profile = profile::load(&recipe).unwrap();
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
        let profile = profile::load(&recipe).unwrap();
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
    let profile = profile::load(&recipe).unwrap();
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
    let profile = profile::load(&recipe).unwrap();
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
    recipe.package.description =
        "An editor with %{name} and 100%% text.\n  Indented text  \n \n".into();
    recipe
        .build_requires
        .rpm
        .push("pkgconfig(example) >= 1.2".into());
    let profile = profile::load(&recipe).unwrap();
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
    let profile = profile::load(&recipe).unwrap();
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
    let profile = profile::load(&recipe).unwrap();
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
