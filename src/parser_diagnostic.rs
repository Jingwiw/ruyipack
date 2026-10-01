// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! First-party parser diagnostic results and their human-readable output.

use std::io::{self, Write};

use serde::Serialize;

use crate::{
    output_cli::{self, HumanLevel},
    source_location::SourceLocation,
};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) span: Option<SourceLocation>,
    pub(crate) message: String,
    pub(crate) notes: Vec<String>,
}

/// Writes every recoverable issue reported by the parser.
pub(crate) fn write(
    diagnostics: &[Diagnostic],
    writer: &mut output_cli::HumanOutput<impl Write>,
) -> io::Result<()> {
    for diagnostic in diagnostics {
        let level = match diagnostic.severity {
            Severity::Warning => HumanLevel::Warn,
            Severity::Error => HumanLevel::Error,
            Severity::Unknown => HumanLevel::Info,
        };
        writer.diagnostic(
            level,
            diagnostic.span.as_ref().map(|span| span.start),
            diagnostic.code.as_deref(),
            format_args!("{}", diagnostic.message),
        )?;
        for note in &diagnostic.notes {
            writer.note(note)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_errors_are_errors_not_warnings_and_notes_stay_plain() {
        let diagnostics =
            [Severity::Warning, Severity::Error, Severity::Unknown].map(|severity| Diagnostic {
                severity,
                code: Some("rpmspec/TEST".into()),
                span: None,
                message: "parser message".into(),
                notes: vec!["detail".into()],
            });
        let mut output = Vec::new();
        write(
            &diagnostics,
            &mut output_cli::HumanOutput::new(&mut output, false),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output,
            "[WARN] spec [rpmspec/TEST]: parser message\n  note: detail\n\
             [ERROR] spec [rpmspec/TEST]: parser message\n  note: detail\n\
             [INFO] spec [rpmspec/TEST]: parser message\n  note: detail\n"
        );
    }
}
