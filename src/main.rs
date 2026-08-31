// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line entry point for read-only SPEC inspection.

use std::{
    env,
    ffi::{OsStr, OsString},
    fmt, fs, io,
    path::PathBuf,
    process::ExitCode,
    string::FromUtf8Error,
};

use rpm_spec::{
    ast::{Span, SpecFile, SpecItem},
    parse_result::{Diagnostic, ParseResult, Severity},
    parser::parse_str_with_spans,
    printer::{self, PrinterConfig},
};

const USAGE: &str = "Usage: ruyipack inspect <SPEC>";

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(parsed) => {
            report(parsed);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            error.exit_code()
        }
    }
}

/// Reads and inspects one SPEC.
fn run(args: impl Iterator<Item = OsString>) -> Result<ParseResult<Span>, CliError> {
    let path = parse_path(args)?;
    let bytes = fs::read(&path).map_err(|source| CliError::Read {
        path: path.clone(),
        source,
    })?;
    let source = String::from_utf8(bytes).map_err(|source| CliError::Utf8 {
        path: path.clone(),
        source,
    })?;

    Ok(parse_str_with_spans(&source))
}

/// Parses the single supported command without adding a second CLI framework.
fn parse_path(mut args: impl Iterator<Item = OsString>) -> Result<PathBuf, CliError> {
    let Some(command) = args.next() else {
        return Err(CliError::Usage);
    };
    if command != OsStr::new("inspect") {
        return Err(CliError::Usage);
    }

    let Some(path) = args.next() else {
        return Err(CliError::Usage);
    };
    if path.is_empty() {
        return Err(CliError::EmptyPath);
    }
    if args.next().is_some() {
        return Err(CliError::Usage);
    }
    Ok(PathBuf::from(path))
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

enum CliError {
    Usage,
    EmptyPath,
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Utf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
}

impl CliError {
    fn exit_code(&self) -> ExitCode {
        match self {
            Self::Usage | Self::EmptyPath => ExitCode::from(2),
            Self::Read { .. } | Self::Utf8 { .. } => ExitCode::FAILURE,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter.write_str(USAGE),
            Self::EmptyPath => formatter.write_str("SPEC path must not be empty"),
            Self::Read { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::Utf8 { path, source } => {
                write!(formatter, "{} is not UTF-8: {source}", path.display())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rpm_spec::ast::{BuildScriptKind, BuildScriptPlacement, Section};

    use super::*;

    static NEXT_TEMP_FILE: AtomicUsize = AtomicUsize::new(0);

    struct TempSpec {
        path: PathBuf,
    }

    impl TempSpec {
        fn write(source: &str) -> Self {
            let id = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("ruyipack-inspect-{}-{id}.spec", std::process::id()));
            fs::write(&path, source).expect("write temporary SPEC");
            Self { path }
        }
    }

    impl Drop for TempSpec {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn inspect_keeps_heredoc_tags_inside_a_placed_build_section() {
        let input = TempSpec::write(
            "\
Name: real
Version: 1

%install -a
cat <<'EOF'
Name: generated
Version: 9
EOF
",
        );
        let args = vec![
            OsString::from("inspect"),
            input.path.clone().into_os_string(),
        ];
        let parsed = match run(args.into_iter()) {
            Ok(parsed) => parsed,
            Err(error) => panic!("inspect failed: {error}"),
        };
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);

        let build_script = parsed.spec.items.iter().find_map(|item| match item {
            SpecItem::Section(section) => match section.as_ref() {
                Section::BuildScript {
                    kind,
                    placement,
                    body,
                    ..
                } => Some((*kind, *placement, body)),
                _ => None,
            },
            _ => None,
        });
        let (kind, placement, body) = build_script.expect("placed install section");
        assert_eq!(kind, BuildScriptKind::Install);
        assert_eq!(placement, BuildScriptPlacement::Append);
        assert!(
            body.lines
                .iter()
                .any(|line| line.literal_str() == Some("Name: generated"))
        );

        let view = main_package_tag_view(parsed.spec);
        let config = PrinterConfig::default().with_preamble_value_column(None);
        assert_eq!(
            printer::print_with(&view, &config),
            "Name: real\nVersion: 1\n"
        );
    }
}
