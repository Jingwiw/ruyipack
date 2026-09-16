// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared Source expression and digest semantics; never evaluates RPM macros.

use std::cell::Cell;
use url::{SyntaxViolation, Url};

#[derive(PartialEq)]
pub(crate) enum Scheme {
    Http,
    Https,
}

/// Checks a Source using only already-known package fields, preserving its spelling.
/// RPM syntax is recognized before URL validation, including escaped literal percent signs.
pub(crate) fn validate_expression(value: &str, fields: &[(&str, &str)]) -> Result<Scheme, String> {
    let resolved = expression::substitute_fields(value, fields)?;
    validate_url(&resolved)
}

/// Checks URL syntax independently of the generator's HTTPS-only publishing policy.
pub(crate) fn validate_url(value: &str) -> Result<Scheme, String> {
    let invalid = || {
        "expected an absolute HTTP or HTTPS URL without whitespace or repaired syntax".to_owned()
    };
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    let repaired = Cell::new(false);
    let capture = |violation| {
        if matches!(
            violation,
            SyntaxViolation::ExpectedDoubleSlash
                | SyntaxViolation::Backslash
                | SyntaxViolation::NonUrlCodePoint
                | SyntaxViolation::PercentDecode
        ) {
            repaired.set(true);
        }
    };
    let parsed = Url::options()
        .syntax_violation_callback(Some(&capture))
        .parse(value)
        .map_err(|_| invalid())?;
    if repaired.get() || parsed.host().is_none() {
        return Err(invalid());
    }
    match parsed.scheme() {
        "http" => Ok(Scheme::Http),
        "https" => Ok(Scheme::Https),
        _ => Err(invalid()),
    }
}

pub(crate) fn validate_sha256(value: &str) -> Result<(), &'static str> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("expected 64 hexadecimal digits")
    }
}

// This adapter is the only Source-expression code that knows rpm-spec's grammar.
mod expression {
    use rpm_spec::{
        ast::{ConditionalMacro, MacroKind, Text, TextSegment},
        parser::{Input, ParserState, text::parse_text},
    };

    pub(super) fn substitute_fields(
        value: &str,
        fields: &[(&str, &str)],
    ) -> Result<String, String> {
        let text = parse(value)?;
        let mut resolved = String::with_capacity(value.len());
        for segment in &text.segments {
            match segment {
                TextSegment::Literal(literal) => resolved.push_str(literal),
                TextSegment::Macro(reference)
                    if matches!(reference.kind, MacroKind::Plain | MacroKind::Braced)
                        && reference.conditional == ConditionalMacro::None
                        && reference.args.is_empty()
                        && reference.with_value.is_none() =>
                {
                    if !matches!(reference.name.as_str(), "name" | "version" | "url") {
                        return Err(format!(
                            "unsupported source macro {:?}; available fields are name, version and url",
                            reference.name
                        ));
                    }
                    let field = fields.iter().find_map(|(name, value)| (*name == reference.name).then_some(*value))
                        .ok_or_else(|| format!("unsupported source macro {:?}: package field is unavailable or ambiguous", reference.name))?;
                    let field = parse(field)?;
                    let literal = field.literal_str().ok_or_else(|| {
                        format!(
                            "unsupported source macro {:?}: package field is not a supported static literal",
                            reference.name
                        )
                    })?;
                    resolved.push_str(literal);
                }
                _ => return Err("unsupported Source expression requires RPM evaluation".into()),
            }
        }
        Ok(resolved)
    }

    fn parse(value: &str) -> Result<Text, String> {
        let state = ParserState::new();
        let (_, text) = parse_text(&state, Input::new(value), &|_| false)
            .map_err(|_| "unsupported or invalid RPM source expression".to_owned())?;
        if !state.diagnostics.borrow().is_empty() {
            return Err("unsupported or invalid RPM source expression".into());
        }
        Ok(text)
    }
}
