// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Direct BuildRequires checks against the selected profile build-system contract.

use super::RuleResult;
use crate::{
    check_report::{Finding, IncompleteReason, SelectedRule, Severity},
    profile::buildsystems,
    source_location::SourceLocation,
};

pub(super) const RULE: SelectedRule = SelectedRule {
    code: "RPK004",
    severity: Severity::Deny,
};

#[derive(Default)]
pub(crate) struct BuildRequirements {
    pub(crate) systems: Vec<(Option<String>, SourceLocation)>,
    pub(crate) direct: Vec<String>,
    pub(crate) uncertain: bool,
}

impl BuildRequirements {
    pub(crate) fn check(&self) -> RuleResult {
        let [(Some(system), span)] = self.systems.as_slice() else {
            return RuleResult::default();
        };
        let Some(contract) = buildsystems::contract(system) else {
            return RuleResult::default();
        };
        let findings: Vec<_> = contract.build_requires.iter().filter(|required| !self.direct.contains(required))
            .map(|required| Finding {
                producer: "ruyipack",
                code: RULE.code,
                severity: if self.uncertain { Severity::Warn } else { RULE.severity },
                message: if self.uncertain {
                    format!("build-requires.rpm: cannot confirm {required:?} required by {system}; conditional or unevaluated requirements need RPM validation")
                } else {
                    format!("build-requires.rpm: declare {required:?} required by {system}")
                },
                span: span.clone(),
            }).collect();
        let incomplete_reasons = if self.uncertain && !findings.is_empty() {
            vec![IncompleteReason::UnresolvedBuildRequirements]
        } else {
            Vec::new()
        };
        RuleResult {
            findings,
            incomplete_reasons,
        }
    }
}
