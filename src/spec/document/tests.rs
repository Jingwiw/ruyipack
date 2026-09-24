// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source-local field changes through the complete document boundary.

use crate::spec::ParsedSpec;
use proptest::prelude::*;
use toml::Value;

use super::Snapshot;

const DEPENDENCIES: &str =
    "Name: demo\nBuildRequires:\tfirst\nBuildRequires:  second\n\n%description\nA demo.\n";
const ED: &str = include_str!("../../../tests/fixtures/ed.spec");

fn capture(source: &str) -> Snapshot {
    Snapshot::capture_selected(&ParsedSpec::parse(source), &[]).unwrap()
}

fn dependencies(values: &[&str]) -> String {
    let snapshot = capture(DEPENDENCIES);
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["rpm"] =
        Value::Array(values.iter().map(|value| (*value).into()).collect());
    snapshot.render(&edited).unwrap()
}

#[test]
fn equal_length_dependency_edits_preserve_line_layout() {
    assert_eq!(
        dependencies(&["first-renamed", "second"]),
        DEPENDENCIES.replace("BuildRequires:\tfirst", "BuildRequires:\tfirst-renamed")
    );
}

#[test]
fn resizing_nonempty_dependencies_rebuilds_only_the_contiguous_group() {
    for (case, values, expected) in [
        (
            "grow",
            &["first", "second", "third"][..],
            "Name: demo\nBuildRequires:  first\nBuildRequires:  second\nBuildRequires:  third\n\n%description\nA demo.\n",
        ),
        (
            "shrink",
            &["first"][..],
            "Name: demo\nBuildRequires:  first\n\n%description\nA demo.\n",
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
fn resizing_separate_dependency_groups_is_rejected() {
    let source = DEPENDENCIES.replace(
        "BuildRequires:  second",
        "# Another group\nBuildRequires:  second",
    );
    let snapshot = capture(&source);
    assert_eq!(snapshot.render(snapshot.document()).unwrap(), source);
    let mut edited = snapshot.document().clone();
    edited["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .push("third".into());
    assert!(
        snapshot
            .render(&edited)
            .unwrap_err()
            .contains("across separate source groups")
    );
}

#[test]
fn copyright_years_and_holders_change_without_overlapping_replacements() {
    let prefix = concat!("# SPDX-FileCopy", "rightText: (C) ");
    let source = format!("{prefix}2025 First Holder\n{prefix}2025 Second Holder\nName: demo\n");
    let snapshot = capture(&source);
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
        assert_eq!(snapshot.render(&edited).unwrap(), expected);
    }
}

#[test]
fn multiline_description_keeps_the_following_section_unchanged() {
    let source = "Name: demo\n%description\nA demo.\n\n%files\n/usr/bin/demo\n";
    let snapshot = capture(source);
    let mut edited = snapshot.document().clone();
    edited["package"]["description"] = "A text editor.\n\nIt keeps its source.\n\n".into();
    assert_eq!(
        snapshot.render(&edited).unwrap(),
        "Name: demo\n%description\nA text editor.\n\nIt keeps its source.\n\n%files\n/usr/bin/demo\n"
    );
}

#[test]
fn ordinary_comment_blocks_can_gain_lines_without_moving() {
    let snapshot = capture(ED);
    let mut edited = snapshot.document().clone();
    edited["spec"]["comments"] = Value::Array(vec![
        "# VCS: No VCS link available\n# Check upstream before changing the archive".into(),
    ]);
    assert_eq!(
        snapshot.render(&edited).unwrap(),
        ED.replace(
            "# VCS: No VCS link available",
            "# VCS: No VCS link available\n# Check upstream before changing the archive"
        )
    );
}

#[test]
fn source_digest_edits_touch_only_the_adjacent_digest() {
    let snapshot = capture(ED);
    let mut edited = snapshot.document().clone();
    edited["sources"]["0"]["sha256"] = "a".repeat(64).into();
    assert_eq!(
        snapshot.render(&edited).unwrap(),
        ED.replace(
            "56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc",
            &"a".repeat(64)
        )
    );
    edited["sources"]["0"]["sha256"] = "not-a-digest".into();
    assert!(
        snapshot
            .render(&edited)
            .unwrap_err()
            .contains("64 hexadecimal digits")
    );
}

#[test]
fn clearing_doc_paths_removes_their_shared_line_once() {
    let source = "Name: demo\n%files\n%doc AUTHORS NEWS\n/usr/bin/demo\n";
    let snapshot = capture(source);
    let mut edited = snapshot.document().clone();
    edited["package"]["files"]["doc"] = Value::Array(Vec::new());
    assert_eq!(
        snapshot.render(&edited).unwrap(),
        "Name: demo\n%files\n/usr/bin/demo\n"
    );
}

#[test]
fn multiple_resized_replacements_preserve_intervening_utf8_bytes() {
    let source = "Name: demo\n# 中间原文\nVersion: 1\nSummary: old\n\n%description\nunchanged\n";
    let snapshot = Snapshot::capture_selected(
        &ParsedSpec::parse(source),
        &[
            "package.name".into(),
            "package.version".into(),
            "package.summary".into(),
        ],
    )
    .unwrap();
    let mut edited = snapshot.document().clone();
    edited["package"]["name"] = "longer-name".into();
    edited["package"]["version"] = "22.333".into();
    edited["package"]["summary"] = "新摘要".into();
    assert_eq!(
        snapshot.render(&edited).unwrap(),
        source
            .replace("Name: demo", "Name: longer-name")
            .replace("Version: 1", "Version: 22.333")
            .replace("Summary: old", "Summary: 新摘要")
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

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
        let snapshot = Snapshot::capture_selected(
            &ParsedSpec::parse(&source),
            &["package.version".into(), "package.summary".into()],
        ).unwrap();
        prop_assert_eq!(snapshot.render(snapshot.document()).unwrap(), source);
        let mut edited = snapshot.document().clone();
        edited["package"]["version"] = new_version.clone().into();
        edited["package"]["summary"] = new_summary.clone().into();
        prop_assert_eq!(snapshot.render(&edited).unwrap(), spec(&new_version, &new_summary));
    }
}
