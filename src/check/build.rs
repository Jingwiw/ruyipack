// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Declared build-system requirements from the openRuyi profile.

use crate::{
    check_report::{Finding, SelectedRule, Severity},
    source_location::SourceLocation,
};
use serde::Deserialize;
use std::sync::OnceLock;

pub(super) const RULE: SelectedRule = SelectedRule {
    code: "RPK004",
    severity: Severity::Deny,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Contract {
    pub(crate) name: String,
    build_requires: Vec<String>,
}

pub(crate) fn autotools() -> &'static Contract {
    static CONTRACT: OnceLock<Contract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        toml::from_str(include_str!(
            "../../profiles/openruyi-v1/buildsystems/autotools.toml"
        ))
        .expect("the embedded Autotools contract must be valid")
    })
}

#[derive(Default)]
pub(crate) struct BuildRequirements {
    pub(crate) systems: Vec<(Option<String>, SourceLocation)>,
    pub(crate) direct: Vec<String>,
    pub(crate) uncertain: bool,
}

impl BuildRequirements {
    pub(crate) fn findings(&self) -> Vec<Finding> {
        let [(Some(system), span)] = self.systems.as_slice() else {
            return Vec::new();
        };
        let contract = autotools();
        if system != &contract.name {
            return Vec::new();
        }
        contract.build_requires.iter().filter(|required| !self.direct.contains(required))
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
            }).collect()
    }
}
