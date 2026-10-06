// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Parsed SPEC syntax and normalized main-package tags.

use super::{ParsedSpec, diagnostic};
use crate::parser_diagnostic::{self, Diagnostic};
use rpm_spec::{
    ast::{CondExpr, CondKind, Span, SpecFile, SpecItem, Tag, TagQualifier, TagValue},
    printer::{self, PrinterConfig},
};
use serde::Serialize;
use std::{
    io::{self, Write},
    path::Path,
};

pub(crate) struct Inspection<'src> {
    source: std::borrow::Cow<'src, str>,
    view: SpecFile<Span>,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> Inspection<'src> {
    pub(crate) fn new(spec: ParsedSpec<'src>) -> Self {
        let mut view = spec.parsed.spec;
        retain_tag_items(&mut view.items);
        let diagnostics = diagnostic::diagnostics(&spec.source, spec.parsed.diagnostics);
        Self {
            source: spec.source,
            view,
            diagnostics,
        }
    }

    pub(crate) fn write_diagnostics(
        &self,
        path: &Path,
        writer: &mut crate::output_cli::HumanOutput<impl Write>,
    ) -> io::Result<()> {
        if !self.diagnostics.is_empty() {
            writer.message(
                crate::output_cli::HumanLevel::Info,
                Some(path),
                format_args!("inspecting SPEC"),
            )?;
        }
        parser_diagnostic::write(&self.diagnostics, writer)
    }

    /// These are declared values, not claims about native macro expansion.
    pub(crate) fn write_identity(&self, writer: &mut impl Write) -> io::Result<()> {
        let config = PrinterConfig::default().with_preamble_value_column(None);
        let contents = printer::print_with(&self.view, &config);
        for line in contents.lines().filter(|line| {
            ["Name:", "Version:", "Release:"]
                .iter()
                .any(|tag| line.starts_with(tag))
        }) {
            writeln!(writer, "build: {line}")?;
        }
        Ok(())
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

    pub(crate) fn write_toml(
        &self,
        path: &Path,
        revision: Option<&str>,
        writer: &mut impl Write,
    ) -> io::Result<()> {
        let sha256 = crate::utf8_file::sha256(&self.source);
        let report = InspectionReport {
            format_version: 2,
            scope: "main-package-syntax",
            not_checked: ["macro-expansion", "native-rpm", "build"],
            tool: crate::tool::identity(),
            input: crate::report::Input {
                display_path: path.to_string_lossy(),
                sha256: Some(&sha256),
                revision,
            },
            parser: ParserIdentity {
                version: env!("RUYIPACK_RPM_SPEC_VERSION"),
                revision: env!("RUYIPACK_RPM_SPEC_REVISION"),
            },
            preamble: items(&self.source, &self.view.items),
            parser_diagnostics: &self.diagnostics,
        };
        crate::report::write(writer, &report)
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
    scope: &'static str,
    not_checked: [&'static str; 3],
    tool: crate::tool::Identity,
    input: crate::report::Input<'a>,
    parser: ParserIdentity,
    preamble: Vec<Item<'a>>,
    parser_diagnostics: &'a [Diagnostic],
}

#[derive(Serialize)]
struct ParserIdentity {
    version: &'static str,
    revision: &'static str,
}

// Output-only containers borrow parser facts; they are not an editable SPEC IR.
// Explicit tag identity avoids TOML dropping Source(None)/Patch(None) variants.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Item<'a> {
    Tag {
        tag: TagIdentity<'a>,
        qualifiers: &'a [TagQualifier],
        lang: Option<&'a str>,
        value: &'a TagValue,
        span: &'a Span,
        raw: Option<&'a str>,
    },
    Conditional {
        branches: Vec<Branch<'a>>,
        otherwise: Option<Vec<Item<'a>>>,
        span: &'a Span,
    },
}

#[derive(Serialize)]
struct TagIdentity<'a> {
    #[serde(serialize_with = "tag_name")]
    name: &'a Tag,
    number: Option<u32>,
}

fn tag_name<S: serde::Serializer>(tag: &Tag, serializer: S) -> Result<S::Ok, S::Error> {
    match tag {
        Tag::Source(_) => serializer.serialize_str("Source"),
        Tag::Patch(_) => serializer.serialize_str("Patch"),
        Tag::NoSource(_) => serializer.serialize_str("NoSource"),
        Tag::NoPatch(_) => serializer.serialize_str("NoPatch"),
        Tag::Other(name) => serializer.serialize_str(name),
        // The remaining pinned parser tags are named unit variants.
        _ => tag.serialize(serializer),
    }
}

#[derive(Serialize)]
struct Branch<'a> {
    kind: CondKind,
    expr: &'a CondExpr<Span>,
    body: Vec<Item<'a>>,
    span: &'a Span,
    /// Original first line only: do not duplicate entire nested branch bodies.
    header: Option<&'a str>,
}

fn items<'a>(source: &'a str, items: &'a [SpecItem<Span>]) -> Vec<Item<'a>> {
    items
        .iter()
        .filter_map(|item| match item {
            SpecItem::Preamble(item) => Some(Item::Tag {
                tag: TagIdentity {
                    name: &item.tag,
                    number: match item.tag {
                        Tag::Source(number) | Tag::Patch(number) => number,
                        Tag::NoSource(number) | Tag::NoPatch(number) => Some(number),
                        _ => None,
                    },
                },
                qualifiers: &item.qualifiers,
                lang: item.lang.as_deref(),
                value: &item.value,
                span: &item.data,
                raw: source.get(item.data.start_byte..item.data.end_byte),
            }),
            SpecItem::Conditional(conditional) => Some(Item::Conditional {
                branches: conditional
                    .branches
                    .iter()
                    .map(|branch| Branch {
                        kind: branch.kind,
                        expr: &branch.expr,
                        body: self::items(source, &branch.body),
                        span: &branch.data,
                        header: source
                            .get(branch.data.start_byte..branch.data.end_byte)
                            .and_then(|text| text.split_inclusive('\n').next()),
                    })
                    .collect(),
                otherwise: conditional
                    .otherwise
                    .as_ref()
                    .map(|body| self::items(source, body)),
                span: &conditional.data,
            }),
            _ => None,
        })
        .collect()
}
