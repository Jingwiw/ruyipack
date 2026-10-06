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
const SUMMARY_RULE: SelectedRule = SelectedRule {
    code: "RPK006",
    severity: Severity::Warn,
};
pub(super) const RULES: [SelectedRule; 3] = [IDENTITY_RULE, URL_RULE, SUMMARY_RULE];

/// Only literal nonempty summaries can be repaired without evaluating macros.
pub(crate) fn fixed_summary(value: &str) -> Option<&str> {
    let value = value.trim_end();
    let fixed = value.strip_suffix('.')?;
    if fixed.ends_with('.') || fixed.trim_end() != fixed {
        return None;
    }
    (!value.contains('%') && fixed != value && !fixed.trim().is_empty()).then_some(fixed)
}

pub(crate) enum Field {
    Name,
    Summary,
    Version,
    Release,
    Url,
}

impl Field {
    fn name(&self) -> &str {
        match self {
            Self::Name => "package.name",
            Self::Summary => "package.summary",
            Self::Version => "package.version",
            Self::Release => "spec.release",
            Self::Url => "package.url",
        }
    }

    /// Validates one literal without macro expansion or URL normalization.
    pub(crate) fn validate(&self, value: &str) -> Result<(), String> {
        self.validate_at(value, self.name())
    }

    /// Uses the caller's authoring path when a shared rule applies to another package.
    pub(crate) fn validate_at(&self, value: &str, field: &str) -> Result<(), String> {
        let result = match self {
            Self::Summary => {
                if value.ends_with('.') {
                    Err("Summary must not end with a period".into())
                } else {
                    Ok(())
                }
            }
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
        result.map_err(|reason| format!("{field}: {reason}"))
    }

    pub(crate) fn finding(&self, literal: Option<&str>, span: SourceLocation) -> Option<Finding> {
        // Static lexical checks make no claim about unevaluated expressions.
        let literal = literal?.trim();
        let error = self.validate(literal).err()?;
        let rule = match self {
            Self::Url => URL_RULE,
            Self::Summary => SUMMARY_RULE,
            _ => IDENTITY_RULE,
        };
        Some(Finding {
            build_requirements: None,
            producer: "ruyipack",
            code: rule.code,
            severity: rule.severity,
            message: error,
            span,
            rule_inputs: Some(vec![self.name().to_owned(), literal.to_owned()]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_repair_preserves_unknown_and_empty_values() {
        assert_eq!(fixed_summary("A useful tool.  "), Some("A useful tool"));
        assert_eq!(fixed_summary("defaults for locale, ..."), None);
        for value in ["A useful tool", "%{upstream_summary}.", "..."] {
            assert_eq!(fixed_summary(value), None);
        }
    }
}
