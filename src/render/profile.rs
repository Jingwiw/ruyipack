// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Embedded openRuyi rendering policy and Autotools requirements.

use super::{RenderError, manifest::Manifest};
use rpm_spec::{
    ast::{DepExpr, Text},
    parser::{ParserState, deps::parse_dep_expr},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct Profile {
    pub(super) spec_license: String,
    pub(super) copyright_holders: Vec<String>,
    pub(super) release: String,
    pub(super) changelog: String,
    pub(super) no_public_vcs_comment: String,
    pub(super) remote_asset_prefix: String,
    pub(super) preamble_value_column: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Contract {
    name: String,
    build_requires: Vec<String>,
}

/// Checks explicit requirements; the contract does not add package dependencies.
pub(super) fn load(manifest: &Manifest) -> Result<Profile, RenderError> {
    let profile = toml::from_str(include_str!("../../profiles/openruyi-v1/profile.toml"))?;
    let contract: Contract = toml::from_str(include_str!(
        "../../profiles/openruyi-v1/buildsystems/autotools.toml"
    ))?;
    if manifest.build.system != contract.name {
        return Err(RenderError::Invalid(format!(
            "build.system: unsupported build system {:?}",
            manifest.build.system
        )));
    }
    let requirements = manifest
        .build_requires
        .rpm
        .iter()
        .map(|entry| {
            parse_dep_expr(&ParserState::new(), entry).map_err(|()| {
                RenderError::Invalid(format!("build-requires.rpm: invalid dependency {entry:?}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for required in contract.build_requires {
        // A conditional or alternative dependency does not guarantee this build tool.
        if !requirements.iter().any(|entry| matches!(entry,
            DepExpr::Atom(atom) if atom.arch.is_none() && atom.name == Text::from(required.as_str())
        )) {
            return Err(RenderError::Invalid(format!(
                "build-requires.rpm: declare {required:?} required by {}",
                contract.name
            )));
        }
    }
    Ok(profile)
}
