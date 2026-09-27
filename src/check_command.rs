// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! File and terminal boundary for the check command.

use std::{io, path::Path};

use crate::{
    check,
    output_cli::{ReportError, ReportFormat, read_source},
    spec::ParsedSpec,
};

/// Checks the selected static rules in one SPEC.
pub(crate) fn run(path: &Path, format: ReportFormat) -> Result<bool, ReportError> {
    let Some(source) = read_source(path, format)? else {
        return Ok(false);
    };
    let report = check::analyze(&ParsedSpec::parse(&source));

    match format {
        ReportFormat::Human => report
            .write_human(path, &mut io::stderr().lock())
            .map_err(ReportError::Stderr)?,
        ReportFormat::Json => report
            .write_json(path, &mut io::stdout().lock())
            .map_err(ReportError::Stdout)?,
    }
    Ok(report.is_success())
}
