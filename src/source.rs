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
    let resolved = crate::spec::expression::substitute_fields(value, fields)?;
    reject_credentials(&resolved)?;
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

/// Authoring policy, separate from syntax checks on unselected existing metadata.
/// Never put the URL or its credentials into the diagnostic.
pub(crate) fn reject_credentials(value: &str) -> Result<(), String> {
    let Ok(parsed) = Url::parse(value) else {
        return Ok(()); // The syntax validator owns malformed-URL diagnostics.
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(
            "URL credentials are not allowed; supply authentication outside the SPEC".into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_sha256(value: &str) -> Result<(), &'static str> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("expected 64 hexadecimal digits")
    }
}
