// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Human-readable RPM parser diagnostic output.

use std::io::{self, Write};

use rpm_spec::parse_result::{Diagnostic as ParserDiagnostic, Severity as ParserSeverity};

/// Writes every recoverable issue reported by the parser.
pub(crate) fn write(diagnostics: &[ParserDiagnostic], writer: &mut impl Write) -> io::Result<()> {
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            ParserSeverity::Warning => "warning",
            ParserSeverity::Error => "error",
            _ => "diagnostic",
        };
        let code = diagnostic
            .code
            .as_deref()
            .map_or_else(String::new, |code| format!("[{code}]"));
        let location = diagnostic.span.map_or_else(String::new, |span| {
            format!(" at {}:{}", span.start_line, span.start_column)
        });

        writeln!(writer, "{severity}{code}{location}: {}", diagnostic.message)?;
        for note in &diagnostic.notes {
            writeln!(writer, "  note: {note}")?;
        }
    }
    Ok(())
}
