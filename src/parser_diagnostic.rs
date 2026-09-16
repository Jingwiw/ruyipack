// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! First-party parser diagnostic results and their human-readable output.

use std::io::{self, Write};

use serde::Serialize;

use crate::source_location::SourceLocation;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    Warning,
    Error,
    Unknown,
}

/// Parser recovery evidence retained separately from static-rule findings.
#[derive(Serialize)]
pub(crate) struct Diagnostic {
    pub(crate) severity: Severity,
    pub(crate) code: Option<String>,
    pub(crate) span: Option<SourceLocation>,
    pub(crate) message: String,
    pub(crate) notes: Vec<String>,
}

/// Writes every recoverable issue reported by the parser.
pub(crate) fn write(diagnostics: &[Diagnostic], writer: &mut impl Write) -> io::Result<()> {
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Unknown => "diagnostic",
        };
        let code = diagnostic
            .code
            .as_deref()
            .map_or_else(String::new, |code| format!("[{code}]"));
        let location = diagnostic.span.as_ref().map_or_else(String::new, |span| {
            format!(" at {}:{}", span.start.0, span.start.1)
        });

        writeln!(writer, "{severity}{code}{location}: {}", diagnostic.message)?;
        for note in &diagnostic.notes {
            writeln!(writer, "  note: {note}")?;
        }
    }
    Ok(())
}
