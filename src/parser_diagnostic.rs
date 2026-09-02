// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Human-readable RPM parser diagnostic output.

use rpm_spec::parse_result::{Diagnostic as ParserDiagnostic, Severity as ParserSeverity};

/// Prints every recoverable issue reported by the parser.
pub(crate) fn print(diagnostics: &[ParserDiagnostic]) {
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

        eprintln!("{severity}{code}{location}: {}", diagnostic.message);
        for note in &diagnostic.notes {
            eprintln!("  note: {note}");
        }
    }
}
