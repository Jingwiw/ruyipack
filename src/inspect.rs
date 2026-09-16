// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! File and terminal boundary for main-package tag inspection.

use crate::{
    spec::{ParsedSpec, inspection::Inspection},
    utf8_file,
};
use clap::ValueEnum;
use std::{io, path::Path};

#[derive(Clone, ValueEnum)]
pub(crate) enum InspectFormat {
    Human,
    Json,
}

/// Reads one SPEC and prints its parser diagnostics and main-package tag view.
pub(crate) fn run(path: &Path, format: InspectFormat) -> Result<(), InspectError> {
    let source = utf8_file::read(path)?;
    let view = Inspection::new(ParsedSpec::parse(&source));
    let mut output = io::stdout().lock();
    match format {
        InspectFormat::Human => {
            view.write_diagnostics(&mut io::stderr().lock())
                .map_err(InspectError::Stderr)?;
            view.write_human(&mut output)
        }
        InspectFormat::Json => view.write_json(path, &mut output),
    }
    .map_err(InspectError::Stdout)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InspectError {
    #[error("{0}")]
    Input(#[from] utf8_file::Utf8FileError),
    #[error("failed to write output to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
}
