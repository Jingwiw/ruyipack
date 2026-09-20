// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! JSON syntax output for read-only SPEC inspection.

use std::{
    io::{self, Write},
    path::Path,
};

use rpm_spec::ast::{Span, SpecFile, SpecItem};
use serde::Serialize;

use crate::parser_diagnostic::Diagnostic;

/// Writes the filtered parser tree and all diagnostics for the same source.
pub(super) fn write(
    path: &Path,
    source: &str,
    view: &SpecFile<Span>,
    diagnostics: &[Diagnostic],
    writer: &mut impl Write,
) -> io::Result<()> {
    let path = path.to_string_lossy();
    let report = Inspection {
        format_version: 1,
        input: InputIdentity {
            display_path: &path,
            sha256: crate::utf8_file::digest(source),
        },
        parser: ParserIdentity {
            version: env!("RUYIPACK_RPM_SPEC_VERSION"),
            revision: env!("RUYIPACK_RPM_SPEC_REVISION"),
        },
        preamble: &view.items,
        parser_diagnostics: diagnostics,
    };
    serde_json::to_writer(&mut *writer, &report)?;
    writeln!(writer)
}

#[derive(Serialize)]
struct Inspection<'a> {
    format_version: u32,
    input: InputIdentity<'a>,
    parser: ParserIdentity,
    preamble: &'a [SpecItem<Span>],
    parser_diagnostics: &'a [Diagnostic],
}

#[derive(Serialize)]
struct InputIdentity<'a> {
    display_path: &'a str,
    sha256: String,
}

#[derive(Serialize)]
struct ParserIdentity {
    version: &'static str,
    revision: &'static str,
}
