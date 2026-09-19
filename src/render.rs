// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Manifest-to-SPEC rendering with the shared static checks.

pub(crate) mod manifest;
pub(crate) mod spec;

use crate::{
    check,
    check_report::CheckReport,
    spec::{ParsedSpec, verify},
};

/// Renders one manifest without file or terminal I/O.
pub(crate) fn run(source: &str) -> Result<RenderedSpec, RenderError> {
    let manifest = manifest::parse(source)?;
    let profile = crate::profile::load()?;
    let contents = spec::render(&manifest, &profile);
    let parsed = ParsedSpec::parse(&contents);
    verify::run(&parsed, &manifest, &profile)?;
    let report = check::analyze(&parsed);
    // A source without a digest is valid but weaker provenance, so gen surfaces
    // it as a warning rather than silently emitting a bare marker.
    let warnings = manifest
        .sources
        .iter()
        .filter(|(_, source)| source.sha256.is_none())
        .map(|(number, _)| {
            format!("sources.{number}: no sha256; rendered a bare #!RemoteAsset without a digest")
        })
        .collect();
    Ok(RenderedSpec {
        name: manifest.package.name,
        contents,
        report,
        warnings,
    })
}
pub(crate) struct RenderedSpec {
    pub(crate) name: String,
    pub(crate) contents: String,
    pub(crate) report: CheckReport,
    pub(crate) warnings: Vec<String>,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum RenderError {
    #[error("{0}")]
    Toml(#[from] toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}
