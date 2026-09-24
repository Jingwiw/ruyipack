// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! File and terminal boundary for main-package tag inspection.

use crate::{
    output_cli::{ReportError, ReportFormat},
    spec::{ParsedSpec, inspection::Inspection},
    utf8_file,
};
use std::{io, path::Path};

/// Reads one SPEC and prints its parser diagnostics and main-package tag view.
pub(crate) fn run(path: &Path, format: ReportFormat) -> Result<(), ReportError> {
    let source = utf8_file::read(path)?;
    let view = Inspection::new(ParsedSpec::parse(&source));
    let mut output = io::stdout().lock();
    match format {
        ReportFormat::Human => {
            view.write_diagnostics(path, &mut io::stderr().lock())
                .map_err(ReportError::Stderr)?;
            view.write_human(&mut output)
        }
        ReportFormat::Json => view.write_json(path, &mut output),
    }
    .map_err(ReportError::Stdout)
}
