// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Required main-package tag presence checks for one RPM SPEC file.

use std::path::Path;

use clap::ValueEnum;
use rpm_spec::{parse_result::Severity as ParserSeverity, parser::parse_str_with_spans};
use rpm_spec_analyzer::{
    config::Config, diagnostic::Severity, registry::builtin_lint_metadata, session::LintSession,
};

use crate::{check_report::CheckReport, check_report::SelectedRule, utf8_file};

const REQUIRED_TAG_LINT_IDS: [&str; 6] =
    ["RPM010", "RPM011", "RPM012", "RPM013", "RPM014", "RPM015"];

/// Output format supported by the check command.
#[derive(Clone, ValueEnum)]
pub(crate) enum CheckFormat {
    Human,
    Json,
}

/// Checks whether one SPEC declares the required main-package tags.
pub(crate) fn run(path: &Path, format: CheckFormat) -> Result<bool, utf8_file::Utf8FileError> {
    let source = utf8_file::read(path)?;
    let report = analyze(&source);

    match format {
        CheckFormat::Human => report.print_human(path),
        CheckFormat::Json => report.print_json(path),
    }
    Ok(report.is_success())
}

/// Runs the selected static checks without file or terminal I/O.
fn analyze(source: &str) -> CheckReport {
    let parsed = parse_str_with_spans(source);
    let (config, selected_rules) = required_tag_policy();

    if parsed
        .diagnostics
        .iter()
        .any(|item| item.severity == ParserSeverity::Error)
    {
        CheckReport::incomplete(source, selected_rules, parsed.diagnostics)
    } else {
        let mut session = LintSession::from_config(&config);
        let findings = session.run(&parsed.spec, source);
        CheckReport::analyzed(source, selected_rules, parsed.diagnostics, findings)
    }
}

/// Resolves the analyzer configuration and selected rules with effective severities.
fn required_tag_policy() -> (Config, Vec<SelectedRule>) {
    let metadata = builtin_lint_metadata();
    let all_ids = metadata.iter().map(|item| item.id).collect::<Vec<_>>();
    let mut config = Config::default();
    config.apply_overrides(&all_ids, Severity::Allow);
    config.apply_overrides(&REQUIRED_TAG_LINT_IDS, Severity::Deny);
    let selected_rules = REQUIRED_TAG_LINT_IDS
        .iter()
        .map(|required_id| {
            let item = metadata
                .iter()
                .find(|item| item.id == *required_id)
                .unwrap_or_else(|| {
                    panic!("required analyzer rule {required_id} is not registered")
                });
            SelectedRule {
                code: item.id,
                severity: config.severity_for(item.id, item.name, item.default_severity),
            }
        })
        .collect();
    (config, selected_rules)
}
