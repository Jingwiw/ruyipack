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

pub(crate) fn diagnostics(
    source: &str,
    diagnostics: Vec<parse_result::Diagnostic>,
) -> Vec<Diagnostic> {
    use parse_result::codes;

    diagnostics
        .into_iter()
        .map(|diagnostic| {
            // The parser also emits these codes from fresh field/body cursors.
            // Without coordinate provenance, their spans are not source locations.
            let span = match diagnostic.code.as_deref() {
                Some(
                    codes::W_STRAY_PERCENT
                    | codes::W_BUILTIN_MISSING_BODY
                    | codes::W_UNTERMINATED_MACRO
                    | codes::W_MACRO_EMPTY_NAME,
                ) => None,
                Some(codes::E_UNTERMINATED_CONDITIONAL) => diagnostic
                    .span
                    .filter(|span| consistent_endpoints(source, *span))
                    .map(location),
                _ => diagnostic.span.map(location),
            }
            .filter(|span| source.get(span.bytes.clone()).is_some());
            Diagnostic {
                severity: match diagnostic.severity {
                    parse_result::Severity::Warning => Severity::Warning,
                    parse_result::Severity::Error => Severity::Error,
                    _ => Severity::Unknown,
                },
                code: diagnostic.code,
                span,
                message: diagnostic.message,
                notes: diagnostic.notes,
            }
        })
        .collect()
}

// Reject disproven locations without guessing the intended diagnostic range.
fn consistent_endpoints(source: &str, span: Span) -> bool {
    if source.get(span.start_byte..span.end_byte).is_none() {
        return false;
    }
    [
        (span.start_byte, (span.start_line, span.start_column)),
        (span.end_byte, (span.end_line, span.end_column)),
    ]
    .into_iter()
    .all(|(offset, expected)| {
        let prefix = &source.as_bytes()[..offset];
        let line = prefix.iter().filter(|&&byte| byte == b'\n').count() + 1;
        let column = prefix
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(offset + 1, |newline| offset - newline);
        u32::try_from(line).ok() == Some(expected.0)
            && u32::try_from(column).ok() == Some(expected.1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_diagnostics_keep_details_and_only_unambiguous_source_coordinates() {
        let converted = diagnostics(
            "abcd\nxyz",
            vec![
                parse_result::Diagnostic::error("invalid syntax")
                    .with_code("rpmspec/E001")
                    .with_span(Span::new(2, 8, 1, 3, 2, 4))
                    .with_note("the recovery context"),
                parse_result::Diagnostic::warning("unlocated warning"),
                parse_result::Diagnostic::warning("unterminated macro")
                    .with_code(parse_result::codes::W_UNTERMINATED_MACRO)
                    .with_span(Span::new(12, 12, 1, 13, 1, 13))
                    .with_note("the macro recovery context"),
            ],
        );
        assert_eq!(
            serde_json::to_value(&converted).unwrap(),
            serde_json::json!([
                {"severity":"error", "code":"rpmspec/E001",
                 "span":{"start_byte":2,"end_byte":8,"start_line":1,"start_column":3,"end_line":2,"end_column":4},
                 "message":"invalid syntax", "notes":["the recovery context"]},
                {"severity":"warning", "code":null, "span":null,
                 "message":"unlocated warning", "notes":[]},
                {"severity":"warning", "code":"rpmspec/W0004", "span":null,
                 "message":"unterminated macro", "notes":["the macro recovery context"]}
            ])
        );
        let mut output = Vec::new();
        crate::parser_diagnostic::write(
            std::path::Path::new("input.spec"),
            &converted,
            &mut output,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "input.spec:1:3: error[rpmspec/E001]: invalid syntax\n  note: the recovery context\ninput.spec: warning: unlocated warning\ninput.spec: warning[rpmspec/W0004]: unterminated macro\n  note: the macro recovery context\n"
        );
    }

    #[test]
    fn diagnostic_ranges_require_byte_boundaries_and_conditional_endpoints() {
        let source = "é\r\nx";
        for (span, bounded, consistent) in [
            (Span::new(0, 2, 1, 1, 1, 3), true, true),
            (Span::new(4, 4, 2, 1, 2, 1), true, true),
            (Span::new(4, 5, 2, 1, 2, 2), true, true),
            (Span::new(5, 5, 2, 2, 2, 2), true, true),
            (Span::new(0, 2, 1, 1, 1, 2), true, false),
            (Span::new(4, 5, 1, 5, 2, 2), true, false),
            (Span::new(1, 2, 1, 2, 1, 3), false, false),
            (Span::new(0, 1, 1, 1, 1, 2), false, false),
            (Span::new(4, 6, 2, 1, 2, 3), false, false),
            (
                Span {
                    start_byte: 5,
                    end_byte: 4,
                    ..Span::default()
                },
                false,
                false,
            ),
        ] {
            for (code, retained) in [
                (parse_result::codes::E_UNTERMINATED_CONDITIONAL, consistent),
                ("rpmspec/E001", bounded),
            ] {
                let converted = diagnostics(
                    source,
                    vec![
                        parse_result::Diagnostic::error("conditional error")
                            .with_code(code)
                            .with_span(span)
                            .with_note("original context"),
                    ],
                );
                assert_eq!(converted[0].span.is_some(), retained, "{span:?}");
                let expected_span = retained.then(|| serde_json::to_value(location(span)).unwrap());
                assert_eq!(
                    serde_json::to_value(&converted).unwrap(),
                    serde_json::json!([{
                        "severity":"error", "code":code, "span":expected_span,
                        "message":"conditional error", "notes":["original context"]
                    }])
                );
            }
        }
    }
}
