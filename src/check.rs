// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared static SPEC checks and rule selection.

pub(crate) mod license;

use crate::{
    check_report::{CheckReport, SelectedRule, Severity},
    parser_diagnostic,
    spec::ParsedSpec,
};

const REQUIRED_TAG_LINT_IDS: [&str; 6] =
    ["RPM010", "RPM011", "RPM012", "RPM013", "RPM014", "RPM015"];

/// Runs the selected static checks without file or terminal I/O.
pub(crate) fn analyze(spec: &ParsedSpec<'_>) -> CheckReport {
    let source = spec.source();
    let mut selected_rules: Vec<_> = REQUIRED_TAG_LINT_IDS
        .iter()
        .map(|&code| SelectedRule {
            code,
            severity: Severity::Deny,
        })
        .collect();
    let diagnostics = spec.diagnostics();

    if diagnostics
        .iter()
        .any(|item| item.severity == parser_diagnostic::Severity::Error)
    {
        selected_rules.push(license::RULE);
        CheckReport::incomplete(source, selected_rules, diagnostics)
    } else {
        let mut findings = spec.findings(&selected_rules);
        selected_rules.push(license::RULE);
        let license = spec.licenses();
        findings.extend(license.findings);
        CheckReport::analyzed(
            source,
            selected_rules,
            diagnostics,
            findings,
            license.unresolved.then_some("unresolved-license"),
        )
    }
}
