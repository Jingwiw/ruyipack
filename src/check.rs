// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Required main-package tag presence checks for one RPM SPEC file.

use std::path::Path;

use rpm_spec::{parse_result::Severity as ParserSeverity, parser::parse_str_with_spans};
use rpm_spec_analyzer::{
    config::Config,
    diagnostic::{Diagnostic, Severity},
    registry::builtin_lint_metadata,
    session::LintSession,
};

use crate::{parser_diagnostic, spec_file};

const REQUIRED_TAG_LINT_IDS: [&str; 6] =
    ["RPM010", "RPM011", "RPM012", "RPM013", "RPM014", "RPM015"];

/// Checks whether one SPEC declares the required main-package tags.
pub(crate) fn run(path: &Path) -> Result<bool, spec_file::SpecReadError> {
    let source = spec_file::read(path)?;
    let parsed = parse_str_with_spans(&source);
    parser_diagnostic::print(&parsed.diagnostics);

    if parsed
        .diagnostics
        .iter()
        .any(|item| item.severity == ParserSeverity::Error)
    {
        eprintln!("error: check incomplete because the SPEC parser reported an error");
        return Ok(false);
    }

    let mut session = required_tag_lint_session();
    let findings = session.run(&parsed.spec, &source);
    print_findings(path, &findings);
    Ok(findings.is_empty())
}

/// Prints the missing-tag findings produced by this command.
fn print_findings(path: &Path, findings: &[Diagnostic]) {
    for finding in findings {
        let severity = match finding.severity {
            Severity::Deny => "error",
            Severity::Warn => "warning",
            Severity::Allow => "diagnostic",
        };
        let span = finding.primary_span;
        eprintln!(
            "{}:{}:{}: {severity}[{}]: {}",
            path.display(),
            span.start_line,
            span.start_column,
            finding.lint_id,
            finding.message
        );
    }
}

/// Builds an analyzer session containing only the required tag rules.
fn required_tag_lint_session() -> LintSession {
    let metadata = builtin_lint_metadata();
    for required_id in REQUIRED_TAG_LINT_IDS {
        assert!(
            metadata.iter().any(|item| item.id == required_id),
            "required analyzer rule {required_id} is not registered"
        );
    }

    let all_ids = metadata.iter().map(|item| item.id).collect::<Vec<_>>();
    let mut config = Config::default();
    config.apply_overrides(&all_ids, Severity::Allow);
    config.apply_overrides(&REQUIRED_TAG_LINT_IDS, Severity::Deny);
    LintSession::from_config(&config)
}
