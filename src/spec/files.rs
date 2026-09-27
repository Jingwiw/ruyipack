// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Bounded native file rows shared by manifest validation and fact comparison.

use rpm_spec::{
    ast::{FileDirective, FileEntry, FilesContent, Span, TextSegment},
    parser::{Input, ParserState, files::parse_files_content},
};

/// A bounded file row, not an arbitrary SPEC section or macro statement.
/// Keep native directive spelling rather than inventing a second flags schema.
pub(crate) fn entry(value: &str) -> Result<FileEntry<Span>, &'static str> {
    fn parse(value: &str) -> Result<FileEntry<Span>, &'static str> {
        let state = ParserState::new();
        let input = format!("{value}\n");
        let (rest, mut items) =
            parse_files_content(&state, Input::new(&input)).map_err(|_| "file entry")?;
        check(
            rest.fragment().is_empty() && state.diagnostics.borrow().is_empty(),
            "file entry",
        )?;
        match items.pop() {
            Some(FilesContent::Entry(entry)) if items.is_empty() => Ok(entry),
            _ => Err("file entry"),
        }
    }
    // rpm-spec 0.4.1 preserves %exclude as macro text instead of a directive.
    // Validate its payload separately while preserving the complete spelling.
    let payload = value.strip_prefix("%exclude ").unwrap_or(value);
    // The pinned parser silently drops unknown config/verify flags.
    // Check only these bounded vocabularies before that information is lost.
    for (prefix, allowed) in [
        ("%config(", &["noreplace", "missingok"][..]),
        (
            "%verify(",
            &[
                "not",
                "md5",
                "filedigest",
                "size",
                "link",
                "user",
                "group",
                "mtime",
                "mode",
                "rdev",
                "caps",
            ][..],
        ),
    ] {
        for rest in payload.split(prefix).skip(1) {
            let (flags, _) = rest.split_once(')').ok_or("file directive flags")?;
            let flags: Vec<_> = if prefix == "%config(" {
                flags.split(',').map(str::trim).collect()
            } else {
                flags.split_whitespace().collect()
            };
            check(
                !flags.is_empty()
                    && flags.iter().enumerate().all(|(i, flag)| {
                        let flag = flag.to_ascii_lowercase();
                        allowed.contains(&flag.as_str())
                            && (flag != "not" || i == 0 && flags.len() > 1)
                    }),
                "file directive flags",
            )?;
        }
    }
    let entry = parse(payload)?;
    check(
        !payload.starts_with('%') || payload.starts_with("%{") || !entry.directives.is_empty(),
        "unsupported file directive",
    )?;
    let relative = entry
        .directives
        .iter()
        .any(|d| matches!(d, FileDirective::Doc | FileDirective::License));
    match &entry.path {
        Some(path) => {
            let valid = match path.path.segments.first() {
                Some(TextSegment::Literal(s)) => relative || s.starts_with('/'),
                Some(TextSegment::Macro(m)) => !matches!(m.kind, rpm_spec::ast::MacroKind::Plain),
                _ => false,
            };
            check(
                valid,
                "file paths must start with / or %{ (except doc/license)",
            )?;
            if !relative && let Some(path) = path.path.literal_str() {
                check(
                    !path.chars().any(char::is_whitespace),
                    "one file path per entry",
                )?;
            }
        }
        None => check(
            matches!(entry.directives.as_slice(), [FileDirective::Defattr(_)]),
            "file directive requires a path",
        )?,
    }
    if payload == value {
        Ok(entry)
    } else {
        parse(value)
    }
}

fn check(matches: bool, reason: &'static str) -> Result<(), &'static str> {
    if matches { Ok(()) } else { Err(reason) }
}
