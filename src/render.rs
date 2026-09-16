// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Manifest-to-SPEC rendering with the shared static checks.

pub(crate) mod manifest;
pub(crate) mod profile;
pub(crate) mod spec;

use crate::{
    check,
    check_report::CheckReport,
    spec::{ParsedSpec, verify},
};

/// Renders one manifest without file or terminal I/O.
pub(crate) fn run(source: &str) -> Result<RenderedSpec, RenderError> {
    let manifest = manifest::parse(source)?;
    let profile = profile::load(&manifest)?;
    let contents = spec::render(&manifest, &profile);
    let parsed = ParsedSpec::parse(&contents);
    verify::run(&parsed, &manifest, &profile)?;
    let report = check::analyze(&parsed);
    Ok(RenderedSpec {
        name: manifest.package.name,
        contents,
        report,
    })
}
pub(crate) struct RenderedSpec {
    pub(crate) name: String,
    pub(crate) contents: String,
    pub(crate) report: CheckReport,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum RenderError {
    #[error("{0}")]
    Toml(#[from] toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}
