// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared static SPEC checks and rule selection.

pub(crate) mod build;
pub(crate) mod license;
pub(crate) mod metadata;

use crate::{
    check_report::{CheckReport, Finding, IncompleteReason, SelectedRule, Severity},
    parser_diagnostic,
    spec::ParsedSpec,
};

/// Findings and unfinished checks are independent facts, not severity conventions.
#[derive(Default)]
pub(crate) struct RuleResult {
    pub(crate) findings: Vec<Finding>,
    pub(crate) incomplete_reasons: Vec<IncompleteReason>,
}

// Required main-package tags for this profile.
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

    let parser_error = diagnostics
        .iter()
        .any(|item| item.severity == parser_diagnostic::Severity::Error);
    let mut findings = if parser_error {
        Vec::new()
    } else {
        spec.findings(&selected_rules)
    };
    selected_rules.push(license::RULE);
    selected_rules.extend(metadata::RULES);
    selected_rules.push(build::RULE);
    if parser_error {
        return CheckReport::incomplete(source, selected_rules, diagnostics);
    }
    let mut incomplete_reasons = Vec::new();
    for result in [spec.licenses(), spec.build_requirements().check()] {
        findings.extend(result.findings);
        incomplete_reasons.extend(result.incomplete_reasons);
    }
    findings.extend(spec.metadata_findings());
    CheckReport::analyzed(
        source,
        selected_rules,
        diagnostics,
        findings,
        incomplete_reasons,
    )
}
