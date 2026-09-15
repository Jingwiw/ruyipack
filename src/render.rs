// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Manifest-to-SPEC rendering with the shared static checks.

mod manifest;
mod profile;
mod spec;
mod verify;

use crate::{check, check_report::CheckReport};
use rpm_spec::parser::parse_str_with_spans;

/// Renders one manifest without file or terminal I/O.
pub(crate) fn run(source: &str) -> Result<RenderedSpec, RenderError> {
    let manifest = manifest::parse(source)?;
    let profile = profile::load(&manifest)?;
    let contents = spec::render(&manifest, &profile);
    let parsed = parse_str_with_spans(&contents);
    verify::run(&contents, &parsed, &manifest, &profile)?;
    let report = check::analyze(&contents, parsed);
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
