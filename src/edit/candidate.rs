// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Pure calculation and static validation of selected SPEC edits.

use super::fields;
use crate::{
    check,
    check_report::CheckReport,
    spec::{ParsedSpec, document::Snapshot},
};
use toml::Table;

pub(super) struct Candidate {
    pub contents: String,
    pub report: CheckReport,
    pub review_triggers: Vec<String>,
}

pub(super) fn prepare(
    snapshot: &Snapshot,
    selection: &[String],
    document: &Table,
) -> Result<Candidate, String> {
    let contents = snapshot.render(document)?;
    let parsed = ParsedSpec::parse(&contents);
    let observed = Snapshot::capture_selected(&parsed, selection)?;
    if observed.document() != document {
        return Err("edited fields did not survive SPEC parsing".into());
    }
    let report = check::analyze(&parsed);
    let mut review_triggers = Vec::new();
    let before = snapshot.document();
    if fields::lookup(before, "package.version") != fields::lookup(document, "package.version") {
        review_triggers.push("package.version".to_owned());
    }
    if let Some(sources) = document.get("sources").and_then(toml::Value::as_table) {
        for number in sources.keys() {
            let field = format!("sources.{number}.url");
            if fields::lookup(before, &field) != fields::lookup(document, &field) {
                review_triggers.push(field);
            }
        }
    }
    Ok(Candidate {
        contents,
        report,
        review_triggers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_repeatable_and_preserve_unselected_bytes() {
        let source = include_str!("../../tests/fixtures/ed.spec");
        let parsed = ParsedSpec::parse(source);
        let selection = vec!["package.version".to_owned()];
        let snapshot = Snapshot::capture_selected(&parsed, &selection).unwrap();
        let unchanged = prepare(&snapshot, &selection, snapshot.document()).unwrap();
        assert_eq!(unchanged.contents, source);
        assert!(unchanged.review_triggers.is_empty());
        let document = fields::assign(
            snapshot.document(),
            &[("package.version".into(), "2".into())],
        )
        .unwrap();
        let first = prepare(&snapshot, &selection, &document).unwrap();
        let second = prepare(&snapshot, &selection, &document).unwrap();
        assert_eq!(
            first.contents,
            source.replace("Version:        1.22.5", "Version:        2")
        );
        assert_eq!(first.contents, second.contents);
        assert_eq!(first.review_triggers, ["package.version"]);
        assert_eq!(first.review_triggers, second.review_triggers);
        let path = std::path::Path::new("ed.spec");
        assert_eq!(
            serde_json::to_value(first.report.structured(path)).unwrap(),
            serde_json::to_value(second.report.structured(path)).unwrap()
        );
        assert!(first.report.is_success());
    }
}
