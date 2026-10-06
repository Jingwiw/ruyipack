// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Pure calculation and static validation of selected SPEC edits.

use crate::{
    check,
    check_report::CheckReport,
    spec::{ParsedSpec, document::Snapshot},
};
use toml::Table;

pub(crate) struct Candidate {
    pub spec: ParsedSpec<'static>,
    pub report: Option<CheckReport>,
    pub changed_fields: Vec<String>,
    pub review_triggers: Vec<String>,
    pub removed_materials: Vec<(std::path::PathBuf, crate::file_digest::Content)>,
    pub source_numbers: Option<std::collections::BTreeMap<u32, u32>>,
}

pub(crate) fn prepare(
    snapshot: &Snapshot<'_>,
    document: &Table,
    defines: &[String],
    run_checks: bool,
) -> Result<Candidate, String> {
    let parsed = snapshot.render(document, defines)?;
    let observed = Snapshot::capture_selected(&parsed, snapshot.selection())?;
    if observed.document() != document {
        return Err("edited fields did not survive SPEC parsing".into());
    }
    let report = run_checks.then(|| check::analyze(&parsed, check::Policy::Authoring, &[]));
    let mut changed_fields = Vec::new();
    collect_changes("", snapshot.document(), document, &mut changed_fields);
    let review_triggers = changed_fields
        .iter()
        .filter(|field| {
            *field == "package.version"
                || (field.starts_with("sources.") && field.ends_with(".url"))
        })
        .cloned()
        .collect();
    Ok(Candidate {
        removed_materials: Vec::new(),
        source_numbers: None,
        spec: parsed,
        report,
        changed_fields,
        review_triggers,
    })
}

// Shape and parsed-value equality have already been checked by prepare.
// Arrays are one editable field, not independent positional facts.
fn collect_changes(prefix: &str, before: &Table, after: &Table, fields: &mut Vec<String>) {
    for (key, value) in after {
        let field = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let (Some(toml::Value::Table(old)), toml::Value::Table(new)) = (before.get(key), value) {
            collect_changes(&field, old, new, fields);
        } else if before.get(key) != Some(value) {
            fields.push(field);
        }
    }
}
