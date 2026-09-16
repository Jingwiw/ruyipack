// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! SPDX expression checks for package License tags.

use rpm_spec::ast::{PreambleItem, Span, SpecFile, Tag, TagValue};
use rpm_spec_analyzer::{diagnostic::Severity, visit::Visit};

use crate::check_report::{Finding, SelectedRule};

pub(super) const RULE: SelectedRule = SelectedRule {
    code: "RPK001",
    severity: Severity::Deny,
};

#[derive(Default)]
pub(super) struct LicenseCheck {
    pub(super) findings: Vec<Finding>,
    pub(super) unresolved: bool,
}

impl LicenseCheck {
    pub(super) fn run(spec: &SpecFile<Span>) -> Self {
        let mut check = Self::default();
        check.visit_spec(spec);
        check
    }
}

impl<'ast> Visit<'ast> for LicenseCheck {
    fn visit_preamble(&mut self, item: &'ast PreambleItem<Span>) {
        if item.tag != Tag::License {
            return;
        }
        let literal = match &item.value {
            TagValue::Text(text) => text.literal_str(),
            _ => None,
        };
        let (severity, message) = match literal {
            Some(value) => match validate_expression(value) {
                Ok(_) => return,
                Err(error) => (
                    Severity::Deny,
                    format!("package.license: invalid or unrecognized SPDX expression: {error}"),
                ),
            },
            None => {
                self.unresolved = true;
                (
                    Severity::Warn,
                    "package.license: SPDX validation requires an evaluated License value".into(),
                )
            }
        };
        self.findings.push(Finding {
            producer: "ruyipack",
            code: RULE.code,
            severity,
            message,
            span: item.data,
        });
    }
}

/// Applies SPDX identifier and operator rules without changing the input text.
fn validate_expression(value: &str) -> Result<(), String> {
    let mode = spdx::ParseMode {
        allow_deprecated: true,
        allow_postfix_plus_on_gpl: true,
        ..spdx::ParseMode::STRICT
    };
    let mut normalized = value.to_owned();
    // The crate's defaults are case-sensitive for IDs and accept lowercase operators.
    // SPDX uses the opposite rules; keep its lexer, identifier data, and grammar parser.
    for token in spdx::lexer::Lexer::new_mode(
        value,
        spdx::ParseMode {
            allow_unknown: true,
            ..mode
        },
    ) {
        let token = token.map_err(|error| error.to_string())?;
        let spelling = &value[token.span.clone()];
        match token.token {
            spdx::lexer::Token::And | spdx::lexer::Token::Or | spdx::lexer::Token::With => {
                if !matches!(spelling, "AND" | "OR" | "WITH") {
                    return Err("SPDX operators must be uppercase AND, OR, or WITH".into());
                }
                if spelling == "WITH"
                    && !(value[..token.span.start]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace)
                        && value[token.span.end..]
                            .chars()
                            .next()
                            .is_some_and(char::is_whitespace))
                {
                    return Err("SPDX WITH requires whitespace on both sides".into());
                }
            }
            spdx::lexer::Token::Unknown(_) => {
                let canonical = spdx::identifiers::LICENSES
                    .iter()
                    .map(|id| id.name)
                    .chain(spdx::identifiers::EXCEPTIONS.iter().map(|id| id.name))
                    .find(|name| name.eq_ignore_ascii_case(spelling));
                if let Some(canonical) = canonical {
                    // ASCII case changes preserve byte positions for parser errors.
                    normalized.replace_range(token.span, canonical);
                }
            }
            _ => {}
        }
    }
    spdx::Expression::parse_mode(&normalized, mode)
        .map(|_| ())
        .map_err(|mut error| {
            error.original = value.to_owned();
            format!("SPDX list {}: {error}", spdx::license_version())
        })
}

#[cfg(test)]
mod tests {
    use super::validate_expression;

    #[test]
    fn spdx_policy_preserves_valid_ids_without_accepting_imprecise_aliases() {
        for value in [
            "MIT",
            "zlib",
            "BSD-2-clause",
            "GPL-2.0",
            "GPL-2.0+",
            "GPL-2.0+ WITH Classpath-exception-2.0",
            "LicenseRef-openRuyi-Public-Domain",
            "GPL-2.0-only WITH classpath-exception-2.0",
        ] {
            assert!(validate_expression(value).is_ok(), "{value}");
        }
        for value in [
            "MIT and Apache-2.0",
            "GPL-2.0+WITH Classpath-exception-2.0",
            "MIT/Apache-2.0",
            "GPLv3+",
            "AGPL-3.0 license",
            "MIT AND",
            "Definitely-Not-A-License",
        ] {
            assert!(validate_expression(value).is_err(), "{value}");
        }
    }
}
