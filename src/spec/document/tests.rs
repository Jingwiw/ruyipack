// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source-local field changes through the complete document boundary.

use crate::spec::ParsedSpec;
use proptest::prelude::*;
use toml::{Table, Value};

use super::Snapshot;

const DEPENDENCIES: &str =
    "Name: demo\nBuildRequires:\tfirst\nBuildRequires:  second\n\n%description\nA demo.\n";
const SPEC: &str = include_str!("../../../tests/fixtures/ed.spec");

fn selected(source: &str, fields: &[String]) -> Snapshot<'static> {
    Snapshot::capture_selected(&ParsedSpec::parse(source), fields)
        .unwrap()
        .into_owned()
}

fn capture(source: &str) -> Snapshot<'static> {
    selected(source, &[])
}

fn render(snapshot: &Snapshot<'_>, edited: &Table) -> Result<String, String> {
    snapshot
        .render(edited, &[])
        .map(|parsed| parsed.source().to_owned())
}

fn dependencies(values: &[&str]) -> String {
    let snapshot = capture(DEPENDENCIES);
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["rpm"] =
        Value::Array(values.iter().map(|value| (*value).into()).collect());
    render(&snapshot, &edited).unwrap()
}

#[test]
fn equal_length_dependency_edits_preserve_line_layout() {
    assert_eq!(
        dependencies(&["first-renamed", "second"]),
        DEPENDENCIES.replace("BuildRequires:\tfirst", "BuildRequires:\tfirst-renamed")
    );
}

#[test]
fn resizing_dependencies_preserves_unchanged_declarations() {
    for (case, values, expected) in [
        (
            "grow",
            &["first", "second", "third"][..],
            "Name: demo\nBuildRequires:\tfirst\nBuildRequires:  second\nBuildRequires:  third\n\n%description\nA demo.\n",
        ),
        (
            "shrink",
            &["first"][..],
            "Name: demo\nBuildRequires:\tfirst\n\n%description\nA demo.\n",
        ),
    ] {
        assert_eq!(dependencies(values), expected, "{case}: {values:?}");
    }
}

#[test]
fn empty_dependencies_remove_the_existing_group() {
    assert_eq!(dependencies(&[]), "Name: demo\n\n%description\nA demo.\n");
}

#[test]
fn dependency_changes_preserve_intervening_comments() {
    let source = DEPENDENCIES.replace(
        "BuildRequires:  second",
        "# 分组原因保留\nBuildRequires:  second",
    );
    let snapshot = capture(&source);
    assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source);
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .push("third".into());
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        source.replace(
            "BuildRequires:  second\n",
            "BuildRequires:  second\nBuildRequires:  third\n"
        )
    );
    edited["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    edited["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        "Name: demo\nBuildRequires:  third\n# 分组原因保留\n\n%description\nA demo.\n"
    );
}

#[test]
fn copyright_years_and_holders_change_without_overlapping_replacements() {
    let prefix = concat!("# SPDX-FileCopy", "rightText: (C) ");
    for (years, first) in [
        ("2025", "First Holder"),
        ("2025, 2026", "First Holder"),
        ("INVALID", "First Holder"),
        ("2025", ""),
    ] {
        let source =
            format!("{prefix}{years} {first}\n{prefix}{years} Second Holder\nName: demo\n");
        let snapshot = capture(&source);
        assert_eq!(
            snapshot.document()["spec"]["copyright-years"].as_str(),
            Some(years)
        );
        assert_eq!(
            snapshot.document()["spec"]["copyright-holders"][0].as_str(),
            Some(first)
        );
        assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source);
        for (holders, expected) in [
            (
                vec!["Updated Holder", "Second Holder"],
                format!(
                    "{prefix}2026-2027 Updated Holder\n{prefix}2026-2027 Second Holder\nName: demo\n"
                ),
            ),
            (
                vec!["Merged Holder"],
                format!("{prefix}2026-2027 Merged Holder\nName: demo\n"),
            ),
        ] {
            let mut edited = snapshot.document().clone();
            edited["spec"]["copyright-years"] = "2026-2027".into();
            edited["spec"]["copyright-holders"] =
                Value::Array(holders.into_iter().map(Value::from).collect());
            assert_eq!(render(&snapshot, &edited).unwrap(), expected);
        }
    }
}

#[test]
fn multiline_description_keeps_the_following_section_unchanged() {
    let source = "Name: demo\n%description\nA demo.\n\n%files\n/usr/bin/demo\n";
    let snapshot = capture(source);
    let mut edited = snapshot.document().clone();
    edited["package"]["description"] = "A text editor.\n\nIt keeps its source.\n\n".into();
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        "Name: demo\n%description\nA text editor.\n\nIt keeps its source.\n\n%files\n/usr/bin/demo\n"
    );
}

#[test]
fn comment_growth_preserves_surrounding_bytes() {
    let snapshot = capture(SPEC);
    let mut edited = snapshot.document().clone();
    edited["spec"]["comments"] = Value::Array(vec![
        "# VCS: No VCS link available\n# Check upstream before changing the archive".into(),
    ]);
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        SPEC.replace(
            "# VCS: No VCS link available",
            "# VCS: No VCS link available\n# Check upstream before changing the archive"
        )
    );
}

#[test]
fn digest_edits_preserve_bare_markers_until_explicitly_filled() {
    let marker =
        "#!RemoteAsset:  sha256:56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc";
    for original in [marker, "#!RemoteAsset"] {
        let source = SPEC.replace(marker, original);
        let snapshot = capture(&source);
        assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source);
        let mut edited = snapshot.document().clone();
        edited["sources"]["0"]["sha256"] = "a".repeat(64).into();
        assert_eq!(
            render(&snapshot, &edited).unwrap(),
            source.replace(
                original,
                &format!("#!RemoteAsset:  sha256:{}", "a".repeat(64))
            )
        );
        for invalid in ["not-a-digest", ""] {
            if invalid.is_empty() && original == "#!RemoteAsset" {
                continue; // Already checked by the unchanged round trip.
            }
            edited["sources"]["0"]["sha256"] = invalid.into();
            assert!(
                render(&snapshot, &edited)
                    .unwrap_err()
                    .contains("64 hexadecimal digits")
            );
        }
    }
}

#[test]
fn clearing_doc_paths_removes_their_shared_line_once() {
    let source = "Name: demo\n%files\n%doc AUTHORS NEWS\n/usr/bin/demo\n";
    let snapshot = capture(source);
    let mut edited = snapshot.document().clone();
    edited["package"]["files"]["doc"] = Value::Array(Vec::new());
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        "Name: demo\n%files\n/usr/bin/demo\n"
    );
}

#[test]
fn repeated_utf8_paths_keep_their_own_ranges() {
    let source = "Name: demo\n%files\n\t%doc\t说明\t说明\n# 保留\n/usr/bin/demo\n";
    let snapshot = selected(source, &["package.files.doc".into()]);
    assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source);
    let mut edited = snapshot.document().clone();
    edited["package"]["files"]["doc"] = Value::Array(vec!["新说明".into(), "second".into()]);
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        "Name: demo\n%files\n\t%doc\t新说明\tsecond\n# 保留\n/usr/bin/demo\n"
    );
}

#[test]
fn multiple_resized_replacements_preserve_intervening_utf8_bytes() {
    let source = "Name: demo\n# 中间原文\nVersion: 1\nSummary: old\n\n%description\nunchanged\n";
    let snapshot = selected(
        source,
        &[
            "package.name".into(),
            "package.version".into(),
            "package.summary".into(),
        ],
    );
    let mut edited = snapshot.document().clone();
    edited["package"]["name"] = "longer-name".into();
    edited["package"]["version"] = "22.333".into();
    edited["package"]["summary"] = "新摘要".into();
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        source
            .replace("Name: demo", "Name: longer-name")
            .replace("Version: 1", "Version: 22.333")
            .replace("Summary: old", "Summary: 新摘要")
    );
}

proptest! {
    #[test]
    fn selected_edits_preserve_unselected_bytes(
        old_version in "[0-9]{1,3}(\\.[0-9]{1,3}){0,2}",
        new_version in "[0-9]{1,3}(\\.[0-9]{1,3}){0,2}",
        old_summary in "[a-z中🙂]{1,24}",
        new_summary in "[a-z中🙂]{1,48}",
        spacing in "[ \\t]{1,4}",
        comment in "[a-z 中🙂]{0,32}",
    ) {
        // Build valid input directly; the oracle never uses captured ranges.
        let spec = |version: &str, summary: &str| format!(
            "Name: demo\n# 保留 {comment}\nVersion:{spacing}{version}\nSummary:{spacing}{summary}\n\n%description\nunchanged\n"
        );
        let source = spec(&old_version, &old_summary);
        let snapshot = selected(&source, &["package.version".into(), "package.summary".into()],
        );
        prop_assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source.as_str());
        let mut edited = snapshot.document().clone();
        edited["package"]["version"] = new_version.clone().into();
        edited["package"]["summary"] = new_summary.clone().into();
        let combined = render(&snapshot, &edited).unwrap();
        prop_assert_eq!(&combined, &spec(&new_version, &new_summary));

        // Only independent literal fields commute. Each step must capture the
        // newly rendered source, whose byte offsets can differ from the original.
        let edit = |source: &str, field: &str, value: &str| {
            let snapshot = selected(source, &[format!("package.{field}")],
            );
            let mut document = snapshot.document().clone();
            document["package"][field] = value.into();
            render(&snapshot, &document).unwrap()
        };
        let version_first = edit(&source, "version", &new_version);
        let summary_first = edit(&source, "summary", &new_summary);
        prop_assert_eq!(edit(&version_first, "summary", &new_summary), combined.as_str());
        prop_assert_eq!(edit(&summary_first, "version", &new_version), combined.as_str());
        prop_assert_eq!(edit(&combined, "version", &new_version), combined.as_str());
        prop_assert_eq!(edit(&combined, "summary", &new_summary), combined.as_str());
    }
}

#[test]
fn crlf_field_values_roundtrip_and_replacements_keep_crlf() {
    let source = SPEC.replace('\n', "\r\n");
    let snapshot = capture(&source);
    let lf = capture(SPEC);
    assert_eq!(snapshot.document(), lf.document());
    assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source);
    for (field, value) in [
        ("package.version", Value::from("1.22.6")),
        (
            "package.description",
            Value::from("第一行\nSecond line\n\n"),
        ),
        ("spec.changelog", Value::from("New entry\n")),
        ("sources.0.sha256", Value::from("a".repeat(64))),
        (
            "spec.comments",
            Value::Array(vec!["# First\n# 第二行".into()]),
        ),
        (
            "package.files.doc",
            Value::Array(vec!["README".into(), "NEWS".into()]),
        ),
        ("package.files.entries", Value::Array(vec![])),
        ("build-requires.rpm", Value::Array(vec!["make".into()])),
        (
            "spec.copyright-holders",
            Value::Array(vec!["One".into(), "Two".into(), "Three".into()]),
        ),
    ] {
        let mut edited = snapshot.document().clone();
        *super::table::lookup_mut(&mut edited, field).unwrap() = value;
        let candidate = crate::spec::candidate::prepare(&snapshot, &edited, &[], false).unwrap();
        assert_eq!(
            candidate.spec.source(),
            render(&lf, &edited).unwrap().replace('\n', "\r\n"),
            "{field}"
        );
    }
    let source = "Name: demo\r\nBuildRequires: first";
    let snapshot = capture(source);
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .push("second".into());
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        "Name: demo\r\nBuildRequires: first\r\nBuildRequires:  second\r\n"
    );
}

#[test]
fn mixed_line_endings_preserve_untouched_bytes_and_reject_bare_controls() {
    let source = "Name: demo\r\nVersion: 1\r\nSummary: unchanged\n%description\r\nold\r\nbody\n";
    let snapshot = capture(source);
    assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source);
    let mut edited = snapshot.document().clone();
    edited["package"]["version"] = "2".into();
    edited["package"]["description"] = "new\nbody\n".into();
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        "Name: demo\r\nVersion: 2\r\nSummary: unchanged\n%description\r\nnew\r\nbody\r\n"
    );
    for source in ["Name: demo\rVersion: 1\n", "Name: demo\0\n"] {
        assert!(
            Snapshot::capture_selected(&ParsedSpec::parse(source), &["package.name".into()])
                .is_err()
        );
    }
}

#[test]
fn dependency_namespaces_and_conditions_roundtrip_without_flattening() {
    let source = "Name: demo\nBuildRequires: pkgconfig(zlib) >= 1.2\nBuildRequires: make\n%if 0%{?feature}\nBuildRequires: cmake(Foo)\n%else\nBuildRequires: fallback\n%endif\n\n%description\nDemo\n";
    let snapshot = selected(source, &["build-requires".into()]);
    assert_eq!(render(&snapshot, snapshot.document()).unwrap(), source);
    assert_eq!(
        snapshot.document()["build-requires"]["pkgconfig"][0].as_str(),
        Some("zlib >= 1.2")
    );
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["if1-else"]["rpm"] = Value::Array(vec![]);
    edited["build-requires"]["if1-then1"]["cmake"] = Value::Array(vec!["Bar".into()]);
    let result = render(&snapshot, &edited).unwrap();
    assert_eq!(
        result,
        source
            .replace("cmake(Foo)", "cmake(Bar)")
            .replace("BuildRequires: fallback\n", "")
    );
    let reparsed = selected(&result, snapshot.selection());
    assert_eq!(reparsed.document(), &edited);
}

#[test]
fn explicit_empty_dependency_namespace_is_inserted_in_selected_branch() {
    let source =
        "Name: demo\n%if 1\nBuildRequires: make\n%else\n# keep\n%endif\n\n%description\nDemo\n";
    let snapshot = selected(source, &["build-requires.if1-else.pkgconfig".into()]);
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["if1-else"]["pkgconfig"] = Value::Array(vec!["zlib".into()]);
    let result = render(&snapshot, &edited).unwrap();
    assert_eq!(
        result,
        source.replace("%else\n", "%else\nBuildRequires:  pkgconfig(zlib)\n")
    );
    assert_eq!(selected(&result, snapshot.selection()).document(), &edited);
}

#[test]
fn first_dependency_preserves_the_header_and_follows_package_metadata() {
    let source = "# SPDX-License-Identifier: MIT\nName: demo\nVersion: 1\n\n%description\nDemo\n";
    let snapshot = selected(source, &["build-requires.pkgconfig".into()]);
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["pkgconfig"] = Value::Array(vec!["zlib".into()]);
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        source.replace(
            "Version: 1\n",
            "Version: 1\nBuildRequires:  pkgconfig(zlib)\n"
        )
    );
}

#[test]
fn supported_projection_keeps_the_parser_failure() {
    let parsed = ParsedSpec::parse("Name: example\n%endif\n");
    let selected = Snapshot::capture_selected(&parsed, &["package.name".into()])
        .err()
        .unwrap();
    let supported = Snapshot::capture_supported(&parsed).err().unwrap();
    assert_eq!(supported, selected);
    assert!(
        supported.contains("`%endif` without matching `%if`"),
        "{supported}"
    );
}

#[test]
fn unrelated_edits_preserve_unresolved_source_but_reject_changed_invalid_url() {
    let source = SPEC.replace(
        "https://ftpmirror.gnu.org/ed/ed-%{version}.tar.lz",
        "%{unknown}",
    );
    let snapshot = selected(&source, &["package.summary".into(), "sources.0.url".into()]);
    let mut edited = snapshot.document().clone();
    edited["package"]["summary"] = "Updated summary".into();
    assert_eq!(
        render(&snapshot, &edited).unwrap(),
        source.replace("A line-oriented text editor", "Updated summary")
    );
    edited["sources"]["0"]["url"] = "https://example.org/with space".into();
    assert!(render(&snapshot, &edited).is_err());
}

#[test]
fn configure_fragments_use_checked_candidates_and_preserve_other_bytes() {
    use crate::spec::candidate;
    for mode in ["prepend", "append", "replace"] {
        let field = format!("build.stages.conf.{mode}");
        let snapshot = selected(SPEC, std::slice::from_ref(&field));
        let mut edited = snapshot.document().clone();
        *super::table::lookup_mut(&mut edited, &field).unwrap() = "echo configured".into();
        let candidate = candidate::prepare(&snapshot, &edited, &[], false).unwrap();
        let header = match mode {
            "prepend" => "%conf -p",
            "append" => "%conf -a",
            _ => "%conf",
        };
        let block = format!("{header}\necho configured\n\n");
        assert_eq!(candidate.spec.source().replace(&block, ""), SPEC);
        let next = selected(candidate.spec.source(), std::slice::from_ref(&field));
        assert_eq!(
            render(&next, next.document()).unwrap(),
            candidate.spec.source()
        );
        *super::table::lookup_mut(&mut edited, &field).unwrap() =
            "echo configured\n%files\n/unselected".into();
        assert!(candidate::prepare(&snapshot, &edited, &[], false).is_err());
    }
}

#[test]
fn configure_ambiguity_is_not_silently_replaced() {
    let field = vec!["build.stages.conf.prepend".into()];
    for block in [
        "%conf -p\necho one\n%conf -p\necho two\n",
        "%if 0\n%conf -p\necho one\n%endif\n",
    ] {
        let text = SPEC.replace("%files", &format!("{block}\n%files"));
        assert!(Snapshot::capture_selected(&ParsedSpec::parse(text), &field).is_err());
    }
}
