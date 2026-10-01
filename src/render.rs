// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Manifest-to-SPEC rendering and independent fact verification.

pub(crate) mod manifest;
pub(crate) mod spec;

use crate::spec::{ParsedSpec, verify};

/// Render and independently verify the exact SPEC, retaining its parsed facts.
/// Policy belongs to the caller; checking a manifest does not run authoring first.
pub(crate) fn run(manifest: &manifest::Manifest) -> Result<ParsedSpec<'static>, RenderError> {
    let profile = crate::profile::load();
    let parsed = ParsedSpec::parse(spec::render(manifest, profile));
    verify::run(&parsed, manifest, profile)?;
    Ok(parsed)
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum RenderError {
    #[error("{0}")]
    Toml(#[from] toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}
