// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Embedded openRuyi rendering policy and Autotools requirements.

use super::{RenderError, manifest::Manifest};
use crate::profile::Profile;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Contract {
    name: String,
    build_requires: Vec<String>,
}

/// Checks explicit requirements; the contract does not add package dependencies.
pub(crate) fn load(manifest: &Manifest) -> Result<Profile, RenderError> {
    let profile = crate::profile::load()?;
    let contract: Contract = toml::from_str(include_str!(
        "../../profiles/openruyi-v1/buildsystems/autotools.toml"
    ))?;
    if manifest.build.system != contract.name {
        return Err(RenderError::Invalid(format!(
            "build.system: unsupported build system {:?}",
            manifest.build.system
        )));
    }
    let requirements =
        crate::spec::expression::direct_dependency_names(&manifest.build_requires.rpm)
            .map_err(RenderError::Invalid)?;
    for required in contract.build_requires {
        // A conditional or alternative dependency does not guarantee this build tool.
        if !requirements.contains(&required) {
            return Err(RenderError::Invalid(format!(
                "build-requires.rpm: declare {required:?} required by {}",
                contract.name
            )));
        }
    }
    Ok(profile)
}
