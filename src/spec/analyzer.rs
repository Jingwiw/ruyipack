// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Adapter for the selected rpm-spec-analyzer rules.

use rpm_spec::ast::{Span, SpecFile};
use rpm_spec_analyzer::{
    config::Config, diagnostic::Severity as AnalyzerSeverity, registry::builtin_lint_metadata,
    session::LintSession,
};

use crate::check_report::{Finding, SelectedRule, Severity};

pub(super) fn run(source: &str, spec: &SpecFile<Span>, rules: &[SelectedRule]) -> Vec<Finding> {
    let metadata = builtin_lint_metadata();
    let all_ids = metadata.iter().map(|item| item.id).collect::<Vec<_>>();
    let mut config = Config::default();
    config.apply_overrides(&all_ids, AnalyzerSeverity::Allow);
    for rule in rules {
        let item = metadata
            .iter()
            .find(|item| item.id == rule.code)
            .unwrap_or_else(|| panic!("required analyzer rule {} is not registered", rule.code));
        let severity = match rule.severity {
            Severity::Allow => AnalyzerSeverity::Allow,
            Severity::Warn => AnalyzerSeverity::Warn,
            Severity::Deny => AnalyzerSeverity::Deny,
        };
        config.apply_overrides(&[item.id], severity);
    }
    LintSession::from_config(&config)
        .run(spec, source)
        .into_iter()
        .map(|diagnostic| Finding {
            producer: "rpm-spec-analyzer",
            code: diagnostic.lint_id,
            severity: match diagnostic.severity {
                AnalyzerSeverity::Allow => Severity::Allow,
                AnalyzerSeverity::Warn => Severity::Warn,
                AnalyzerSeverity::Deny => Severity::Deny,
            },
            message: diagnostic.message,
            span: super::diagnostic::location(diagnostic.primary_span),
        })
        .collect()
}
