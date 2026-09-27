// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Original SPEC bytes, a fixed field selection, and their editable TOML view.
//! Capture owns AST/range mapping; render validates replacements and preserves other bytes.

mod capture;
mod render;
pub(crate) mod table;

use std::{borrow::Cow, collections::BTreeMap, ops::Range};
use toml::Table;

struct Scalar {
    field: String,
    range: Range<usize>,
    multiline: bool,
}

#[derive(Default)]
struct List {
    items: Vec<Range<usize>>,
    lines: Vec<Range<usize>>,
    prefix: String,
}

struct Copyright {
    years: Vec<Range<usize>>,
    holders: List,
}

/// Original bytes plus replacement ranges for one fixed field selection.
/// Capturing proves a field can be located, not that its old value is valid;
/// rendering validates the selected replacement so damaged values remain repairable.
pub(crate) struct Snapshot<'src> {
    // Long-lived edits own their source; candidate verification only borrows it.
    source: Cow<'src, str>,
    document: Table,
    selection: Vec<String>,
    package_context: BTreeMap<String, String>,
    scalars: Vec<Scalar>,
    digest_markers: BTreeMap<String, Range<usize>>,
    lists: BTreeMap<String, List>,
    copyright: Option<Copyright>,
}

impl Snapshot<'_> {
    pub(crate) fn into_owned(self) -> Snapshot<'static> {
        Snapshot {
            source: Cow::Owned(self.source.into_owned()),
            document: self.document,
            selection: self.selection,
            package_context: self.package_context,
            scalars: self.scalars,
            digest_markers: self.digest_markers,
            lists: self.lists,
            copyright: self.copyright,
        }
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn selection(&self) -> &[String] {
        &self.selection
    }

    pub(crate) fn document(&self) -> &Table {
        &self.document
    }

    fn selects(&self, field: &str) -> bool {
        selected(&self.selection, field)
    }
}

// Selection chooses author fields, not RPM execution branches or expanded values.
fn selected(selection: &[String], field: &str) -> bool {
    selection.is_empty()
        || selection.iter().any(|selected| {
            selected == field
                || field
                    .strip_prefix(selected)
                    .is_some_and(|tail| tail.starts_with('.'))
                || selected
                    .strip_prefix(field)
                    .is_some_and(|tail| tail.starts_with('.'))
        })
}

fn validate_text(value: &str, field: &str, multiline: bool) -> Result<(), String> {
    if value.contains(['\r', '\0']) || (!multiline && value.contains('\n')) {
        return Err(format!("{field}: CR, NUL or scalar newline is unsupported"));
    }
    if !multiline && (value.is_empty() || value.trim() != value) {
        return Err(format!(
            "{field}: expected nonempty value without surrounding whitespace"
        ));
    }
    Ok(())
}

fn validate_file_path(value: &str, field: &str) -> Result<(), String> {
    validate_text(value, field, false)?;
    if value.chars().any(char::is_whitespace) || value.contains(['\'', '"', '\\', '#']) {
        return Err(format!(
            "{field}: quoted, escaped, whitespace or comment-bearing paths are unsupported"
        ));
    }
    Ok(())
}

fn validate_comments(value: &str) -> Result<(), String> {
    validate_text(value, "spec.comments", true)?;
    if value.is_empty()
        || value.ends_with('\n')
        || value.lines().any(|line| !line.starts_with('#'))
        || value.contains("RemoteAsset")
        || value.contains("SPDX-FileCopyrightText:")
        || value.contains("SPDX-FileContributor:")
        || value.contains(concat!("SPDX-License-", "Identifier:"))
    {
        return Err(
            "spec.comments: expected ordinary complete # comment lines without final newline"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
