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
    check_report::{CheckReport, SelectedRule, Severity},
    parser_diagnostic,
    spec::ParsedSpec,
};

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
    let license = spec.licenses();
    findings.extend(license.findings);
    findings.extend(spec.metadata_findings());
    let build = spec.build_requirements().findings();
    let unresolved_build = build
        .iter()
        .any(|finding| finding.severity == Severity::Warn);
    findings.extend(build);
    CheckReport::analyzed(
        source,
        selected_rules,
        diagnostics,
        findings,
        license
            .unresolved
            .then_some("unresolved-license")
            .or(unresolved_build.then_some("unresolved-build-requirements")),
    )
}
