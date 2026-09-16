// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared static SPEC checks and rule selection.

use rpm_spec::{ast::Span, parse_result::ParseResult};

mod analyzer;
mod license;
mod syntax;

use crate::{
    check_report::{CheckReport, SelectedRule, Severity},
    parser_diagnostic, syntax_diagnostic,
};

const REQUIRED_TAG_LINT_IDS: [&str; 6] =
    ["RPM010", "RPM011", "RPM012", "RPM013", "RPM014", "RPM015"];

/// Runs the selected static checks without file or terminal I/O.
///
/// `parsed` must have been produced from `source`.
pub(crate) fn analyze(source: &str, parsed: ParseResult<Span>) -> CheckReport {
    let mut selected_rules: Vec<_> = REQUIRED_TAG_LINT_IDS
        .iter()
        .map(|&code| SelectedRule {
            code,
            severity: Severity::Deny,
        })
        .collect();
    let analyzer = analyzer::Analyzer::new(&selected_rules);
    selected_rules.push(license::RULE);
    let diagnostics = syntax_diagnostic::diagnostics(parsed.diagnostics);

    if diagnostics
        .iter()
        .any(|item| item.severity == parser_diagnostic::Severity::Error)
    {
        CheckReport::incomplete(source, selected_rules, diagnostics)
    } else {
        let mut findings = analyzer.run(source, &parsed.spec);
        let license = syntax::license(&parsed.spec);
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
