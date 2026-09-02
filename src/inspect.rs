// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only inspection of main-package tags in an RPM SPEC file.

use std::path::Path;

use rpm_spec::{
    ast::{Span, SpecFile, SpecItem},
    parse_result::ParseResult,
    parser::parse_str_with_spans,
    printer::{self, PrinterConfig},
};

use crate::{parser_diagnostic, spec_file};

/// Reads one SPEC and prints its parser diagnostics and main-package tag view.
pub(crate) fn run(path: &Path) -> Result<(), spec_file::SpecReadError> {
    let source = spec_file::read(path)?;
    report(parse_str_with_spans(&source));
    Ok(())
}

/// Prints parser diagnostics and the normalized main-package tag view.
fn report(parsed: ParseResult<Span>) {
    let view = main_package_tag_view(parsed.spec);
    parser_diagnostic::print(&parsed.diagnostics);
    let config = PrinterConfig::default().with_preamble_value_column(None);
    let output = printer::print_with(&view, &config);
    if output.is_empty() {
        println!("No main-package tags found.");
    } else {
        print!("{output}");
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
