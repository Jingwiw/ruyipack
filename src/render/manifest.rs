// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Typed authoring input for SPEC previews.

use super::RenderError;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct Manifest {
    pub(super) spec: SpecMetadata,
    pub(super) package: Package,
    pub(super) sources: BTreeMap<String, Source>,
    pub(super) build: Build,
    pub(super) build_requires: BuildRequires,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct SpecMetadata {
    pub(super) copyright_years: String,
    pub(super) contributors: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Package {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) summary: String,
    pub(super) license: String,
    pub(super) url: String,
    pub(super) description: String,
    pub(super) vcs: Vcs,
    pub(super) files: Files,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct Vcs {
    pub(super) no_public_repository: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    pub(super) url: String,
    pub(super) sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Build {
    pub(super) system: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BuildRequires {
    pub(super) rpm: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Files {
    #[serde(default)]
    pub(super) license: Vec<String>,
    #[serde(default)]
    pub(super) doc: Vec<String>,
    pub(super) entries: Vec<String>,
}

/// Reads the supported authoring shape without evaluating RPM macros.
pub(super) fn parse(source: &str) -> Result<Manifest, RenderError> {
    let manifest: Manifest = toml::from_str(source)?;
    if manifest.sources.len() != 1 || !manifest.sources.contains_key("0") {
        return Err(RenderError::Invalid(
            "sources: this renderer requires exactly sources.0".into(),
        ));
    }
    if !manifest.package.vcs.no_public_repository {
        return Err(RenderError::Invalid(
            "package.vcs: this renderer requires no-public-repository = true".into(),
        ));
    }
    Ok(manifest)
}
