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
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub(super) const RULE: SelectedRule = SelectedRule {
    code: "RPK004",
    severity: Severity::Deny,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Contract {
    pub(crate) name: String,
    pub(crate) build_requires: Vec<String>,
    // Default action per stage, sourced from openRuyi (see each contract file
    // header for the exact file, commit, and hash). The init template renders
    // these as guidance; nothing else consumes them yet.
    #[serde(default)]
    pub(crate) stages: Vec<StageAction>,
}

/// One stage's default action and optional guidance, as declared by openRuyi.
/// Serialized straight into the init template context, so the field names match
/// the template. Notes stay `Option` (serialized as null, not skipped) because
/// the template runs under strict undefined checks.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StageAction {
    pub(crate) name: String,
    pub(crate) action: String,
    #[serde(default)]
    pub(crate) inline_note: Option<String>,
    #[serde(default)]
    pub(crate) full_note: Option<String>,
}

/// One embedded build-system contract source, parsed on first use.
struct Embedded {
    toml: &'static str,
    cache: OnceLock<Contract>,
}

impl Embedded {
    const fn new(toml: &'static str) -> Self {
        Self {
            toml,
            cache: OnceLock::new(),
        }
    }

    fn get(&self) -> &Contract {
        self.cache
            .get_or_init(|| toml::from_str(self.toml).expect("embedded contract must be valid"))
    }
}

// Each supported system maps to a profile TOML. Adding a system means adding a
// contract file and one entry here, not a new lookup path.
static CONTRACTS: [Embedded; 3] = [
    Embedded::new(include_str!(
        "../../profiles/openruyi-v1/buildsystems/autotools.toml"
    )),
    Embedded::new(include_str!(
        "../../profiles/openruyi-v1/buildsystems/cmake.toml"
    )),
    Embedded::new(include_str!(
        "../../profiles/openruyi-v1/buildsystems/meson.toml"
    )),
];

/// Returns the contract for a declared system name, if one is supported.
pub(crate) fn contract(name: &str) -> Option<&'static Contract> {
    CONTRACTS
        .iter()
        .map(Embedded::get)
        .find(|contract| contract.name == name)
}

/// Names of every supported build system, in declaration order.
pub(crate) fn systems() -> impl Iterator<Item = &'static str> {
    CONTRACTS
        .iter()
        .map(|embedded| embedded.get().name.as_str())
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
        let Some(contract) = contract(system) else {
            return Vec::new();
        };
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
