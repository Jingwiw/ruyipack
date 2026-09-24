// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Parsed SPEC syntax and normalized main-package tags.

use super::{ParsedSpec, diagnostic};
use crate::parser_diagnostic::{self, Diagnostic};
use rpm_spec::{
    ast::{Span, SpecFile, SpecItem},
    printer::{self, PrinterConfig},
};
use serde::Serialize;
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
        let mut view = spec.parsed.spec;
        retain_tag_items(&mut view.items);
        Self {
            source: spec.source,
            view,
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
        let path = path.to_string_lossy();
        let report = InspectionReport {
            format_version: 1,
            input: InputIdentity {
                display_path: &path,
                sha256: crate::utf8_file::digest(self.source),
            },
            parser: ParserIdentity {
                version: env!("RUYIPACK_RPM_SPEC_VERSION"),
                revision: env!("RUYIPACK_RPM_SPEC_REVISION"),
            },
            preamble: &self.view.items,
            parser_diagnostics: &self.diagnostics,
        };
        serde_json::to_writer(&mut *writer, &report)?;
        writeln!(writer)
    }
}

/// Keeps every main-package tag and the conditional structure around it.
fn retain_tag_items(items: &mut Vec<SpecItem<Span>>) {
    items.retain_mut(|item| match item {
        SpecItem::Preamble(_) => true,
        SpecItem::Conditional(conditional) => {
            let mut contains_tags = false;

            for branch in &mut conditional.branches {
                retain_tag_items(&mut branch.body);
                contains_tags |= !branch.body.is_empty();
            }
            if let Some(otherwise) = conditional.otherwise.as_mut() {
                retain_tag_items(otherwise);
                contains_tags |= !otherwise.is_empty();
            }

            contains_tags
        }
        _ => false,
    });
}

#[derive(Serialize)]
struct InspectionReport<'a> {
    format_version: u32,
    input: InputIdentity<'a>,
    parser: ParserIdentity,
    preamble: &'a [SpecItem<Span>],
    parser_diagnostics: &'a [Diagnostic],
}

#[derive(Serialize)]
struct InputIdentity<'a> {
    display_path: &'a str,
    sha256: String,
}

#[derive(Serialize)]
struct ParserIdentity {
    version: &'static str,
    revision: &'static str,
}
