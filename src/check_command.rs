// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! File and terminal boundary for the check command.

use std::{io, path::Path};

use clap::ValueEnum;
use rpm_spec::parser::parse_str_with_spans;

use crate::{check, utf8_file};

/// Output format supported by the check command.
#[derive(Clone, ValueEnum)]
pub(crate) enum CheckFormat {
    Human,
    Json,
}

/// Checks whether one SPEC declares the required main-package tags.
pub(crate) fn run(path: &Path, format: CheckFormat) -> Result<bool, CheckError> {
    let source = utf8_file::read(path)?;
    let report = check::analyze(&source, parse_str_with_spans(&source));

    match format {
        CheckFormat::Human => report
            .write_human(path, &mut io::stderr().lock())
            .map_err(CheckError::Stderr)?,
        CheckFormat::Json => report
            .write_json(path, &mut io::stdout().lock())
            .map_err(CheckError::Stdout)?,
    }
    Ok(report.is_success())
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CheckError {
    #[error("{0}")]
    Input(#[from] utf8_file::Utf8FileError),
    #[error("failed to write output to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
}
