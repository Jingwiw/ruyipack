// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared static SPEC checks, rule selection, and the check command boundary.

pub(crate) mod build;
pub(crate) mod license;
pub(crate) mod metadata;

use std::{io, path::Path};

use crate::{
    check_report::{CheckReport, Finding, IncompleteReason, SelectedRule, Severity},
    output_cli::{ReportError, ReportFormat, read_source},
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

// Missing or malformed source digests warn in static checks, so unrelated edits
// remain possible. Authoring a digest still requires a valid SHA-256.
pub(crate) const SOURCE_DIGEST_RULE: SelectedRule = SelectedRule {
    code: "RPK005",
    severity: Severity::Warn,
};

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
    selected_rules.push(SOURCE_DIGEST_RULE);
    if parser_error {
        return CheckReport::incomplete(source, selected_rules, diagnostics);
    }
    let local = spec.local_checks();
    findings.extend(local.findings);
    CheckReport::analyzed(
        source,
        selected_rules,
        diagnostics,
        findings,
        local.incomplete_reasons,
    )
}

/// Checks the selected static rules in one SPEC.
pub(crate) fn run(path: &Path, format: ReportFormat) -> Result<bool, ReportError> {
    let Some(source) = read_source(path, format)? else {
        return Ok(false);
    };
    let report = analyze(&ParsedSpec::parse(&source));

    match format {
        ReportFormat::Human => report
            .write_human(path, &mut io::stderr().lock())
            .map_err(ReportError::Stderr)?,
        ReportFormat::Json => report
            .write_json(path, &mut io::stdout().lock())
            .map_err(ReportError::Stdout)?,
    }
    Ok(report.is_success())
}
