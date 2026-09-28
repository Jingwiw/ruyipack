// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Pure calculation and static validation of selected SPEC edits.

use crate::spec::document::table;
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
    pub source_hashes: Option<crate::source::SourceHashes>,
}

pub(super) fn prepare(
    snapshot: &Snapshot<'_>,
    document: &Table,
    defines: &[String],
) -> Result<Candidate, String> {
    let contents = snapshot.render(document, defines)?;
    let parsed = ParsedSpec::parse(&contents);
    let observed = Snapshot::capture_selected(&parsed, snapshot.selection())?;
    if observed.document() != document {
        return Err("edited fields did not survive SPEC parsing".into());
    }
    let report = check::analyze(&parsed, check::Policy::Authoring, &[]);
    let mut review_triggers = Vec::new();
    let before = snapshot.document();
    if table::lookup(before, "package.version") != table::lookup(document, "package.version") {
        review_triggers.push("package.version".to_owned());
    }
    if let Some(sources) = document.get("sources").and_then(toml::Value::as_table) {
        for number in sources.keys() {
            let field = format!("sources.{number}.url");
            if table::lookup(before, &field) != table::lookup(document, &field) {
                review_triggers.push(field);
            }
        }
    }
    Ok(Candidate {
        contents,
        report,
        review_triggers,
        source_hashes: None,
    })
}
