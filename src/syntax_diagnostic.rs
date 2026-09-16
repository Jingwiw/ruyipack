// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Conversion from parser coordinates and recovery diagnostics.

use rpm_spec::{ast::Span, parse_result};

use crate::{
    parser_diagnostic::{Diagnostic, Severity},
    source_location::SourceLocation,
};

pub(crate) fn location(span: Span) -> SourceLocation {
    SourceLocation {
        bytes: span.start_byte..span.end_byte,
        start: (span.start_line, span.start_column),
        end: (span.end_line, span.end_column),
    }
}

pub(crate) fn diagnostics(diagnostics: Vec<parse_result::Diagnostic>) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| Diagnostic {
            severity: match diagnostic.severity {
                parse_result::Severity::Warning => Severity::Warning,
                parse_result::Severity::Error => Severity::Error,
                _ => Severity::Unknown,
            },
            code: diagnostic.code,
            span: diagnostic.span.map(location),
            message: diagnostic.message,
            notes: diagnostic.notes,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_diagnostics_keep_severity_notes_and_all_source_coordinates() {
        let converted = diagnostics(vec![
            parse_result::Diagnostic::error("invalid syntax")
                .with_code("rpmspec/E001")
                .with_span(Span::new(2, 8, 1, 3, 2, 4))
                .with_note("the recovery context"),
            parse_result::Diagnostic::warning("unlocated warning"),
        ]);
        assert_eq!(
            serde_json::to_value(&converted).unwrap(),
            serde_json::json!([
                {"severity":"error", "code":"rpmspec/E001",
                 "span":{"start_byte":2,"end_byte":8,"start_line":1,"start_column":3,"end_line":2,"end_column":4},
                 "message":"invalid syntax", "notes":["the recovery context"]},
                {"severity":"warning", "code":null, "span":null,
                 "message":"unlocated warning", "notes":[]}
            ])
        );
        let mut output = Vec::new();
        crate::parser_diagnostic::write(&converted, &mut output).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "error[rpmspec/E001] at 1:3: invalid syntax\n  note: the recovery context\nwarning: unlocated warning\n"
        );
    }
}
