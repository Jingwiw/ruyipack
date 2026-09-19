// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Parsed SPEC syntax and normalized main-package tags.

mod json;
use super::{ParsedSpec, diagnostic};
use crate::parser_diagnostic::{self, Diagnostic};
use rpm_spec::{
    ast::{Span, SpecFile, SpecItem},
    printer::{self, PrinterConfig},
};
use std::{
    io::{self, Write},
    path::Path,
};

pub(crate) struct Inspection<'src> {
    source: &'src str,
    view: SpecFile<Span>,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> Inspection<'src> {
    pub(crate) fn new(spec: ParsedSpec<'src>) -> Self {
        Self {
            source: spec.source,
            view: main_package_tag_view(spec.parsed.spec),
            diagnostics: diagnostic::diagnostics(spec.source, spec.parsed.diagnostics),
        }
    }

    pub(crate) fn write_diagnostics(&self, path: &Path, writer: &mut impl Write) -> io::Result<()> {
        parser_diagnostic::write(path, &self.diagnostics, writer)
    }

    pub(crate) fn write_human(&self, writer: &mut impl Write) -> io::Result<()> {
        let config = PrinterConfig::default().with_preamble_value_column(None);
        let contents = printer::print_with(&self.view, &config);
        if contents.is_empty() {
            writeln!(writer, "No main-package tags found.")
        } else {
            writer.write_all(contents.as_bytes())
        }
    }

    pub(crate) fn write_json(&self, path: &Path, writer: &mut impl Write) -> io::Result<()> {
        json::write(path, self.source, &self.view, &self.diagnostics, writer)
    }
}

/// Keeps every main-package tag and the conditional structure around it.
fn main_package_tag_view(mut spec: SpecFile<Span>) -> SpecFile<Span> {
    spec.items = retain_tag_items(spec.items);
    spec
}

/// Filters one AST item list while preserving source order.
fn retain_tag_items(items: Vec<SpecItem<Span>>) -> Vec<SpecItem<Span>> {
    items.into_iter().filter_map(retain_tag_item).collect()
}

/// Keeps one tag or one conditional containing tags.
fn retain_tag_item(item: SpecItem<Span>) -> Option<SpecItem<Span>> {
    match item {
        item @ SpecItem::Preamble(_) => Some(item),
        SpecItem::Conditional(mut conditional) => {
            let mut contains_tags = false;

            for branch in &mut conditional.branches {
                branch.body = retain_tag_items(std::mem::take(&mut branch.body));
                contains_tags |= !branch.body.is_empty();
            }
            if let Some(otherwise) = conditional.otherwise.as_mut() {
                *otherwise = retain_tag_items(std::mem::take(otherwise));
                contains_tags |= !otherwise.is_empty();
            }

            contains_tags.then_some(SpecItem::Conditional(conditional))
        }
        _ => None,
    }
}
