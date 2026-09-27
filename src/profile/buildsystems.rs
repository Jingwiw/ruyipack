// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Build-system contracts: declared requirements and default stage actions.
//!
//! Each supported system has a TOML file in `profiles/openruyi/buildsystems`
//! and an entry in `CONTRACTS`. These supply CLI choices, init guidance, and
//! the RPK004 requirement check. Stage actions are guidance, not shell scripts
//! executed by RuyiPack: the generated BuildSystem tag selects target RPM macros.
//! Each TOML records its policy/macro source. An empty requirement list means
//! this tool has no common requirement contract, not that the system needs no tools.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Contract {
    pub(crate) name: String,
    pub(crate) build_requires: Vec<String>,
    // Default stage actions shown as guidance in init templates.
    #[serde(default)]
    pub(crate) stages: Vec<StageAction>,
}

/// Stage guidance for init. Missing notes serialize as null for strict templates.
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

const SOURCES: [&str; 3] = [
    include_str!("../../profiles/openruyi/buildsystems/autotools.toml"),
    include_str!("../../profiles/openruyi/buildsystems/cmake.toml"),
    include_str!("../../profiles/openruyi/buildsystems/meson.toml"),
];
static CONTRACTS: LazyLock<[Contract; 3]> = LazyLock::new(|| {
    SOURCES.map(|source| toml::from_str(source).expect("embedded contract must be valid"))
});

/// Returns the contract for a declared system name, if one is supported.
pub(crate) fn contract(name: &str) -> Option<&'static Contract> {
    CONTRACTS.iter().find(|contract| contract.name == name)
}

/// Identifies the exact embedded contract selected by a manifest.
pub(crate) fn contract_identity(name: &str) -> Option<crate::profile::Identity> {
    CONTRACTS
        .iter()
        .zip(SOURCES)
        .find(|(contract, _)| contract.name == name)
        .map(|(_, source)| crate::profile::Identity {
            name: format!("openruyi/buildsystems/{name}"),
            sha256: crate::utf8_file::sha256(source),
        })
}

/// Names of every supported build system, in declaration order.
pub(crate) fn systems() -> impl Iterator<Item = &'static str> {
    CONTRACTS.iter().map(|contract| contract.name.as_str())
}
