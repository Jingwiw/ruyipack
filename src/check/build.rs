// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Direct `BuildRequires` checks against the selected profile build-system contract.

use super::RuleResult;
use crate::{
    check_report::{BuildRequirementsEvidence, Finding, SelectedRule, Severity},
    profile::buildsystems,
    source_location::SourceLocation,
};

pub(super) const RULE: SelectedRule = SelectedRule {
    code: "RPK004",
    severity: Severity::Warn,
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
        let mut declared = self.direct.clone();
        declared.sort();
        declared.dedup();
        let suggested = contract
            .build_requires
            .iter()
            .filter(|required| !declared.contains(required))
            .cloned()
            .collect::<Vec<_>>();
        if suggested.is_empty() {
            return RuleResult::default();
        }
        let message = format!(
            "BuildRequires: consider explicitly declaring {} (BuildSystem={system}{})",
            suggested.join(", "),
            if self.uncertain {
                "; not statically confirmed"
            } else {
                ""
            }
        );
        let mut inputs = vec![system.clone()];
        inputs.extend(declared.iter().cloned());
        RuleResult {
            findings: vec![Finding {
                producer: "ruyipack",
                code: RULE.code,
                severity: RULE.severity,
                message,
                span: span.clone(),
                rule_inputs: (!self.uncertain).then_some(inputs),
                build_requirements: Some(BuildRequirementsEvidence {
                    build_system: system.clone(),
                    declared,
                    suggested,
                    uncertain: self.uncertain,
                }),
            }],
            ..RuleResult::default()
        }
    }
}
