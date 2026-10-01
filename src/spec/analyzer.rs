// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! The six required-tag rules, without constructing the analyzer's full registry.

use rpm_spec_analyzer::{Lint, rules::missing_tag::*};

use crate::check_report::{Finding, SelectedRule, Severity};

/// Absent AST means parsing failed: report the selection without inventing missing tags.
/// These rules use parser-owned root spans and need no source/config/profile setup.
pub(crate) fn required_tags(
    spec: Option<&super::ParsedSpec<'_>>,
) -> (Vec<SelectedRule>, Vec<Finding>) {
    let lints: [&mut dyn Lint; 6] = [
        &mut MissingNameTag::new(),
        &mut MissingVersionTag::new(),
        &mut MissingReleaseTag::new(),
        &mut MissingLicenseTag::new(),
        &mut MissingSummaryTag::new(),
        &mut MissingUrlTag::new(),
    ];
    let mut rules = Vec::new();
    let mut findings = Vec::new();
    for lint in lints {
        // openRuyi requires all six, including URL (upstream defaults it to Warn).
        rules.push(SelectedRule {
            code: lint.metadata().id,
            severity: Severity::Deny,
        });
        if let Some(spec) = spec {
            lint.visit_spec(&spec.parsed.spec);
            findings.extend(
                lint.take_diagnostics()
                    .into_iter()
                    .map(|diagnostic| Finding {
                        producer: "rpm-spec-analyzer",
                        code: diagnostic.lint_id,
                        severity: Severity::Deny,
                        message: diagnostic.message,
                        span: super::diagnostic::location(diagnostic.primary_span),
                        rule_inputs: None,
                    }),
            );
        }
    }
    (rules, findings)
}
