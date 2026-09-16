// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only inspection of main-package tags in an RPM SPEC file.

use std::{
    io::{self, Write},
    path::Path,
};

mod json;

use clap::ValueEnum;
use rpm_spec::{
    ast::{Span, SpecFile, SpecItem},
    parser::parse_str_with_spans,
    printer::{self, PrinterConfig},
};

use crate::{parser_diagnostic, syntax_diagnostic, utf8_file};

#[derive(Clone, ValueEnum)]
pub(crate) enum InspectFormat {
    Human,
    Json,
}

/// Reads one SPEC and prints its parser diagnostics and main-package tag view.
pub(crate) fn run(path: &Path, format: InspectFormat) -> Result<(), InspectError> {
    let source = utf8_file::read(path)?;
    let parsed = parse_str_with_spans(&source);
    let view = main_package_tag_view(parsed.spec);
    let diagnostics = syntax_diagnostic::diagnostics(parsed.diagnostics);
    let mut output = io::stdout().lock();
    match format {
        InspectFormat::Human => {
            parser_diagnostic::write(&diagnostics, &mut io::stderr().lock())
                .map_err(InspectError::Stderr)?;
            let config = PrinterConfig::default().with_preamble_value_column(None);
            let contents = printer::print_with(&view, &config);
            if contents.is_empty() {
                writeln!(output, "No main-package tags found.")
            } else {
                output.write_all(contents.as_bytes())
            }
        }
        InspectFormat::Json => json::write(path, &source, &view, &diagnostics, &mut output),
    }
    .map_err(InspectError::Stdout)
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

#[derive(Debug, thiserror::Error)]
pub(crate) enum InspectError {
    #[error("{0}")]
    Input(#[from] utf8_file::Utf8FileError),
    #[error("failed to write output to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
}
