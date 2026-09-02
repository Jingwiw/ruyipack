// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only inspection of main-package tags in an RPM SPEC file.

use std::{
    error, fmt, fs, io,
    path::{Path, PathBuf},
    string::FromUtf8Error,
};

use rpm_spec::{
    ast::{Span, SpecFile, SpecItem},
    parse_result::{Diagnostic, ParseResult, Severity},
    parser::parse_str_with_spans,
    printer::{self, PrinterConfig},
};

/// Reads one SPEC and prints its parser diagnostics and main-package tag view.
pub(crate) fn run(path: &Path) -> Result<(), Error> {
    let bytes = fs::read(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let source = String::from_utf8(bytes).map_err(|source| Error::Utf8 {
        path: path.to_path_buf(),
        source,
    })?;

    report(parse_str_with_spans(&source));
    Ok(())
}

/// Prints parser diagnostics and the normalized main-package tag view.
fn report(parsed: ParseResult<Span>) {
    let view = main_package_tag_view(parsed.spec);
    print_parser_diagnostics(&parsed.diagnostics);
    let config = PrinterConfig::default().with_preamble_value_column(None);
    let output = printer::print_with(&view, &config);
    if output.is_empty() {
        println!("No main-package tags found.");
    } else {
        print!("{output}");
    }
}

/// Prints every recoverable issue reported by the parser.
fn print_parser_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
            _ => "diagnostic",
        };
        let code = diagnostic
            .code
            .as_deref()
            .map_or_else(String::new, |code| format!("[{code}]"));
        let location = diagnostic.span.map_or_else(String::new, |span| {
            format!(" at {}:{}", span.start_line, span.start_column)
        });

        eprintln!("{severity}{code}{location}: {}", diagnostic.message);
        for note in &diagnostic.notes {
            eprintln!("  note: {note}");
        }
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

#[derive(Debug)]
pub(crate) enum Error {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Utf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::Utf8 { path, source } => {
                write!(formatter, "{} is not UTF-8: {source}", path.display())
            }
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Utf8 { source, .. } => Some(source),
        }
    }
}
