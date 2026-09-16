// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Literal package identity and project URL checks.

use crate::{
    check_report::{Finding, SelectedRule, Severity},
    source_location::SourceLocation,
};

const IDENTITY_RULE: SelectedRule = SelectedRule {
    code: "RPK002",
    severity: Severity::Deny,
};
const URL_RULE: SelectedRule = SelectedRule {
    code: "RPK003",
    severity: Severity::Deny,
};
pub(super) const RULES: [SelectedRule; 2] = [IDENTITY_RULE, URL_RULE];

pub(crate) enum Field {
    Name,
    Version,
    Release,
    Url,
}

impl Field {
    fn name(&self) -> &str {
        match self {
            Self::Name => "package.name",
            Self::Version => "package.version",
            Self::Release => "spec.release",
            Self::Url => "package.url",
        }
    }

    /// Validates one literal without macro expansion or URL normalization.
    pub(crate) fn validate(&self, value: &str) -> Result<(), String> {
        let result = match self {
            Self::Name => {
                if value
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
                {
                    Ok(())
                } else {
                    Err("expected an RPM package name".into())
                }
            }
            Self::Version | Self::Release => {
                // https://rpm.org/docs/6.0.x/manual/spec.html#version
                if !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._+~^".contains(&b))
                {
                    Ok(())
                } else {
                    Err(
                        "expected an RPM version or release using letters, digits, . _ + ~ ^"
                            .into(),
                    )
                }
            }
            Self::Url => crate::source::validate_url(value).map(|_| ()),
        };
        result.map_err(|reason| format!("{}: {reason}", self.name()))
    }

    pub(crate) fn finding(&self, literal: Option<&str>, span: SourceLocation) -> Option<Finding> {
        // Static lexical checks make no claim about unevaluated expressions.
        let error = self.validate(literal?.trim()).err()?;
        let rule = if matches!(self, Self::Url) {
            URL_RULE
        } else {
            IDENTITY_RULE
        };
        Some(Finding {
            producer: "ruyipack",
            code: rule.code,
            severity: rule.severity,
            message: error,
            span,
        })
    }
}
