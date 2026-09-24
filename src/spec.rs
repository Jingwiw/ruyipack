// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Parsed SPEC source and private integrations with the RPM syntax libraries.

mod analyzer;
mod diagnostic;
pub(crate) mod document;
pub(crate) mod expression;
pub(crate) mod inspection;
mod syntax;
pub(crate) mod verify;

use crate::{
    check::RuleResult,
    check_report::{Finding, SelectedRule},
    parser_diagnostic::Diagnostic,
};
use rpm_spec::{ast::Span, parse_result::ParseResult, parser::parse_str_with_spans};

/// Keeps parser ranges tied to the exact source that produced them.
pub(crate) struct ParsedSpec<'src> {
    source: &'src str,
    parsed: ParseResult<Span>,
}

impl<'src> ParsedSpec<'src> {
    pub(crate) fn parse(source: &'src str) -> Self {
        Self {
            source,
            parsed: parse_str_with_spans(source),
        }
    }

    pub(crate) fn source(&self) -> &str {
        self.source
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        diagnostic::diagnostics(self.source, self.parsed.diagnostics.clone())
    }

    pub(crate) fn findings(&self, rules: &[SelectedRule]) -> Vec<Finding> {
        analyzer::run(self.source, &self.parsed.spec, rules)
    }

    pub(crate) fn local_checks(&self) -> RuleResult {
        syntax::check(&self.parsed.spec, self.source)
    }
}
