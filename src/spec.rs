// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Parsed SPEC source and private integrations with the RPM syntax libraries.

pub(crate) mod analyzer;
pub(crate) mod candidate;
mod checks;
mod diagnostic;
pub(crate) mod document;
pub(crate) mod expression;
pub(crate) mod files;
pub(crate) mod inspection;
pub(crate) mod sources;
pub(crate) mod verify;

use crate::{check::RuleResult, parser_diagnostic::Diagnostic};
use std::{borrow::Cow, cell::OnceCell};

use rpm_spec::{ast::Span, parse_result::ParseResult, parser::parse_str_with_spans};

/// Keeps parser ranges tied to the exact source that produced them.
pub(crate) struct ParsedSpec<'src> {
    source: Cow<'src, str>,
    parsed: ParseResult<Span>,
    sources: OnceCell<Result<sources::Resolution, String>>,
}

impl<'src> ParsedSpec<'src> {
    pub(crate) fn parse(source: impl Into<Cow<'src, str>>) -> Self {
        let source = source.into();
        let parsed = parse_str_with_spans(&source);
        Self {
            source,
            parsed,
            sources: OnceCell::new(),
        }
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        diagnostic::diagnostics(&self.source, self.parsed.diagnostics.clone())
    }

    pub(crate) fn policy_checks(&self, defines: &[String]) -> RuleResult {
        checks::run(self, defines)
    }
}

/// RPM implicit numbering advances past the greatest explicit number seen.
/// None preserves uncertainty caused by hidden declarations (or overflow).
fn source_number(next: &mut Option<u32>, explicit: Option<u32>) -> Option<u32> {
    let number = explicit.or(*next);
    *next = next.and_then(|next| number.and_then(|n| n.checked_add(1).map(|n| next.max(n))));
    number
}
