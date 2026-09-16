// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! A bounded author-field projection with source-local replacements.

use rpm_spec::{
    ast::{FileDirective, FilesContent, Section, Span, SpecItem, Tag},
    parse_result::{ParseResult, Severity},
};
use std::{collections::BTreeMap, ops::Range};
use toml::{Table, Value};

use super::ParsedSpec;
use crate::edit::fields::{lookup, lookup_mut, validate_shape};

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

pub(crate) struct Snapshot {
    source: String,
    document: Table,
    selection: Vec<String>,
    source_fields: BTreeMap<String, String>,
    scalars: Vec<Scalar>,
    lists: BTreeMap<String, List>,
    copyright: Option<Copyright>,
}

impl Snapshot {
    pub(crate) fn capture(spec: &ParsedSpec<'_>) -> Result<Self, String> {
        Self::capture_selected(spec, &[])
    }

    pub(crate) fn capture_selected(
        spec: &ParsedSpec<'_>,
        selection: &[String],
    ) -> Result<Self, String> {
        let source = spec.source;
        let parsed = &spec.parsed;
        if source.contains(['\r', '\0']) {
            return Err("source: CR and NUL are unsupported".into());
        }
        if parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            return Err("source: parser errors prevent a complete mapping".into());
        }
        let mut snapshot = Self {
            source: source.to_owned(),
            document: Table::new(),
            selection: selection.to_vec(),
            source_fields: source_fields(source, parsed),
            scalars: Vec::new(),
            lists: BTreeMap::new(),
            copyright: None,
        };
        snapshot.list("spec.contributors", "# SPDX-FileContributor: ");
        snapshot.list("spec.comments", "");
        snapshot.list("build-requires.rpm", "BuildRequires:  ");
        let profile = crate::profile::load().map_err(|error| error.to_string())?;
        let mut coverage = Vec::new();
        let mut comments = Vec::new();
        let mut consumed_assets = Vec::new();
        let mut sections = Vec::new();
        for (index, item) in parsed.spec.items.iter().enumerate() {
            match item {
                SpecItem::Blank => {}
                SpecItem::Comment(comment) => {
                    let range = checked_range(source, comment.data)?;
                    coverage.push(range.clone());
                    comments.push(range);
                }
                SpecItem::Preamble(item) => {
                    if !selection.is_empty()
                        && !preamble_field(&item.tag).is_some_and(|field| snapshot.selects(&field))
                    {
                        continue;
                    }
                    let range = checked_range(source, item.data)?;
                    coverage.push(range.clone());
                    if !item.qualifiers.is_empty() || item.lang.is_some() {
                        return Err(
                            "preamble: qualifiers and localized fields are unsupported".into()
                        );
                    }
                    let raw = line(source, &range)?;
                    let (name, value) = raw.split_once(':').ok_or("preamble: missing colon")?;
                    let value_start = range.start + name.len() + 1 + value.len()
                        - value.trim_start_matches([' ', '\t']).len();
                    let value_end = range.start + raw.trim_end_matches([' ', '\t']).len();
                    let value_range = value_start..value_end;
                    let (field, expected) = match &item.tag {
                        Tag::Name => ("package.name".to_owned(), "Name".to_owned()),
                        Tag::Version => ("package.version".to_owned(), "Version".to_owned()),
                        Tag::Release => ("spec.release".to_owned(), "Release".to_owned()),
                        Tag::Summary => ("package.summary".to_owned(), "Summary".to_owned()),
                        Tag::License => ("package.license".to_owned(), "License".to_owned()),
                        Tag::URL => ("package.url".to_owned(), "URL".to_owned()),
                        Tag::Other(name) if name.eq_ignore_ascii_case("BuildSystem") => {
                            ("build.system".to_owned(), name.clone())
                        }
                        Tag::BuildRequires => {
                            if !name.trim().eq_ignore_ascii_case("BuildRequires") {
                                return Err("build-requires.rpm: AST/source header mismatch".into());
                            }
                            snapshot.list_item("build-requires.rpm", value_range, range)?;
                            continue;
                        }
                        Tag::Source(number) => {
                            let number = number.unwrap_or(0);
                            let identity = format!("sources.{number}");
                            if lookup(&snapshot.document, &format!("{identity}.url")).is_some() {
                                return Err(format!("{identity}: duplicate Source identity"));
                            }
                            let expected = match &item.tag {
                                Tag::Source(Some(number)) => format!("Source{number}"),
                                _ => "Source".to_owned(),
                            };
                            let Some(SpecItem::Comment(previous)) =
                                index.checked_sub(1).and_then(|i| parsed.spec.items.get(i))
                            else {
                                return Err(format!(
                                    "{identity}.sha256: missing adjacent RemoteAsset comment"
                                ));
                            };
                            let asset = checked_range(source, previous.data)?;
                            if asset.end != range.start {
                                return Err(format!(
                                    "{identity}.sha256: RemoteAsset must be immediately adjacent"
                                ));
                            }
                            let asset_text = &source[asset.clone()];
                            let prefix = profile.remote_asset_prefix.as_str();
                            let hash = asset_text
                                .strip_prefix(prefix)
                                .and_then(|s| s.strip_suffix('\n'))
                                .ok_or_else(|| {
                                    format!("{identity}.sha256: unsupported RemoteAsset syntax")
                                })?;
                            valid_hash(hash, &format!("{identity}.sha256"))?;
                            snapshot.scalar(
                                &format!("{identity}.sha256"),
                                asset.start + prefix.len()..asset.end - 1,
                                false,
                            )?;
                            consumed_assets.push(asset);
                            (format!("{identity}.url"), expected)
                        }
                        _ => return Err(format!("preamble: unsupported tag {:?}", item.tag)),
                    };
                    if !name.trim().eq_ignore_ascii_case(&expected) {
                        return Err(format!("{field}: AST/source header mismatch"));
                    }
                    snapshot.scalar(&field, value_range, false)?;
                }
                SpecItem::Section(section) => {
                    let selected_section =
                        section_field(section).is_some_and(|field| snapshot.selects(field));
                    let selected_comments =
                        matches!(section.as_ref(), Section::Files { subpkg: None, .. })
                            && snapshot.selects("spec.comments");
                    if !(selection.is_empty() || selected_section || selected_comments) {
                        continue;
                    }
                    let (name, span) = match section.as_ref() {
                        Section::Description { subpkg: None, data, .. } => ("description", *data),
                        Section::Files { subpkg: None, file_lists, data, .. } if file_lists.is_empty() => ("files", *data),
                        Section::Changelog { data, .. } => ("changelog", *data),
                        _ => return Err("section: only simple main-package description, files and changelog are supported".into()),
                    };
                    if sections.contains(&name) {
                        return Err(format!("{name}: duplicate section"));
                    }
                    sections.push(name);
                    let range = checked_range(source, span)?;
                    coverage.push(range.clone());
                    let raw = &source[range.clone()];
                    let header_end = raw.find('\n').map_or(raw.len(), |i| i + 1);
                    if raw[..header_end].trim() != format!("%{name}") {
                        return Err(format!("{name}: unsupported section header"));
                    }
                    let body = range.start + header_end..range.end;
                    match section.as_ref() {
                        Section::Description { .. } => {
                            snapshot.scalar("package.description", body, true)?
                        }
                        Section::Changelog { .. } => {
                            snapshot.scalar("spec.changelog", body, true)?
                        }
                        Section::Files { content, .. } => {
                            snapshot.files(content, body, &mut comments)?
                        }
                        _ => unreachable!(),
                    }
                }
                SpecItem::Conditional(conditional) if !selection.is_empty() => {
                    for branch in &conditional.branches {
                        reject_selected_conditional(source, &branch.body, selection)?;
                    }
                    if let Some(items) = &conditional.otherwise {
                        reject_selected_conditional(source, items, selection)?;
                    }
                }
                _ if !selection.is_empty() => {}
                _ => return Err(
                    "source: conditional, macro definition, include or statement is unsupported"
                        .into(),
                ),
            }
        }
        if selection.is_empty() {
            check_coverage(source, 0..source.len(), &mut coverage)?;
        }
        snapshot.comments(comments, &consumed_assets)?;
        if selection.is_empty() && lookup(&snapshot.document, "package.name").is_none() {
            return Err("package.name: required main package is missing".into());
        }
        for scalar in &snapshot.scalars {
            if scalar.field.starts_with("sources.") && scalar.field.ends_with(".url") {
                valid_source_url(
                    string(&snapshot.document, &scalar.field)?,
                    &scalar.field,
                    &snapshot.source_fields,
                )?;
            }
        }
        Ok(snapshot)
    }

    pub(crate) fn document(&self) -> &Table {
        &self.document
    }

    pub(crate) fn render(&self, edited: &Table) -> Result<String, String> {
        validate_shape(&self.document, edited)?;
        let mut fields = self.source_fields.clone();
        for name in ["name", "version", "url"] {
            if let Some(value) = lookup(edited, &format!("package.{name}")).and_then(Value::as_str)
            {
                fields.insert(name.to_owned(), value.to_owned());
            }
        }
        let mut changes = Vec::new();
        for scalar in &self.scalars {
            let value = string(edited, &scalar.field)?;
            valid_text(value, &scalar.field, scalar.multiline)?;
            if scalar.field.starts_with("sources.") && scalar.field.ends_with(".url") {
                valid_source_url(value, &scalar.field, &fields)?;
            }
            if scalar.field.ends_with(".sha256") {
                valid_hash(value, &scalar.field)?;
            }
            if value != &self.source[scalar.range.clone()] {
                changes.push((scalar.range.clone(), value.to_owned()));
            }
        }
        for (field, list) in &self.lists {
            let values = strings(edited, field)?;
            if field == "spec.comments" && !values.is_empty() && values.len() != list.items.len() {
                return Err("spec.comments: edit lines within an existing comment block; adding or regrouping blocks is unsupported".into());
            }
            for value in &values {
                if field == "spec.comments" {
                    valid_comments(value)?;
                } else {
                    valid_text(value, field, false)?;
                    if field.starts_with("package.files.") {
                        valid_path(value, field)?;
                    }
                }
            }
            self.replace_list(list, &values, field, &list.prefix, &mut changes)?;
        }
        if let Some(copyright) = &self.copyright {
            let years = if self.selects("spec.copyright-years") {
                string(edited, "spec.copyright-years")?
            } else {
                &self.source[copyright.years[0].clone()]
            };
            crate::spec_metadata::validate_years(years)?;
            let holders = if self.selects("spec.copyright-holders") {
                strings(edited, "spec.copyright-holders")?
            } else {
                copyright
                    .holders
                    .items
                    .iter()
                    .map(|range| &self.source[range.clone()])
                    .collect()
            };
            if holders.is_empty() {
                return Err("spec.copyright-holders: cannot remove every holder while copyright-years is present".into());
            }
            for holder in &holders {
                valid_text(holder, "spec.copyright-holders", false)?;
            }
            if holders.len() == copyright.holders.items.len() {
                for range in &copyright.years {
                    if years != &self.source[range.clone()] {
                        changes.push((range.clone(), years.to_owned()));
                    }
                }
            }
            let prefix = format!("# SPDX-FileCopyrightText: (C) {years} ");
            self.replace_list(
                &copyright.holders,
                &holders,
                "spec.copyright-holders",
                &prefix,
                &mut changes,
            )?;
        }
        changes.sort_by_key(|(range, _)| range.start);
        if changes
            .windows(2)
            .any(|pair| pair[0].0.end > pair[1].0.start)
        {
            return Err("edit: overlapping replacements".into());
        }
        let mut output = self.source.clone();
        for (range, value) in changes.into_iter().rev() {
            output.replace_range(range, &value);
        }
        Ok(output)
    }

    fn selects(&self, field: &str) -> bool {
        selected(&self.selection, field)
    }

    fn scalar(&mut self, field: &str, range: Range<usize>, multiline: bool) -> Result<(), String> {
        if !self.selects(field) {
            return Ok(());
        }
        let value = self
            .source
            .get(range.clone())
            .ok_or_else(|| format!("{field}: invalid value span"))?;
        valid_text(value, field, multiline)?;
        insert(&mut self.document, field, Value::String(value.to_owned()))?;
        self.scalars.push(Scalar {
            field: field.to_owned(),
            range,
            multiline,
        });
        Ok(())
    }

    fn list(&mut self, field: &str, prefix: &str) {
        if !self.selects(field) {
            return;
        }
        self.lists.entry(field.to_owned()).or_insert_with(|| List {
            prefix: prefix.to_owned(),
            ..List::default()
        });
        // All callers initialize each fixed field only once.
        insert(&mut self.document, field, Value::Array(Vec::new()))
            .expect("unique fixed list field");
    }

    fn list_item(
        &mut self,
        field: &str,
        range: Range<usize>,
        line: Range<usize>,
    ) -> Result<(), String> {
        if !self.selects(field) {
            return Ok(());
        }
        let value = self
            .source
            .get(range.clone())
            .ok_or_else(|| format!("{field}: invalid list span"))?;
        valid_text(value, field, field == "spec.comments")?;
        let list = self
            .lists
            .get_mut(field)
            .ok_or_else(|| format!("{field}: list not initialized"))?;
        list.items.push(range);
        if list.lines.last() != Some(&line) {
            list.lines.push(line);
        }
        lookup_mut(&mut self.document, field)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| format!("{field}: expected array"))?
            .push(Value::String(value.to_owned()));
        Ok(())
    }

    fn files(
        &mut self,
        content: &[FilesContent<Span>],
        body: Range<usize>,
        comments: &mut Vec<Range<usize>>,
    ) -> Result<(), String> {
        for (field, prefix) in [("license", "%license "), ("doc", "%doc "), ("entries", "")] {
            self.list(&format!("package.files.{field}"), prefix);
        }
        let mut coverage = Vec::new();
        for item in content {
            match item {
                FilesContent::Blank => {}
                FilesContent::Comment(comment) => {
                    let range = checked_range(&self.source, comment.data)?;
                    coverage.push(range.clone());
                    comments.push(range);
                }
                FilesContent::Entry(entry) => {
                    if !self.selects("package.files") {
                        continue;
                    }
                    let relevant = if entry.directives.contains(&FileDirective::Doc) {
                        self.selects("package.files.doc")
                    } else if entry.directives.contains(&FileDirective::License) {
                        self.selects("package.files.license")
                    } else {
                        self.selects("package.files.entries")
                    };
                    if !relevant {
                        continue;
                    }
                    let range = checked_range(&self.source, entry.data)?;
                    coverage.push(range.clone());
                    let raw = line(&self.source, &range)?;
                    let leading = raw.len() - raw.trim_start_matches([' ', '\t']).len();
                    let text = &raw[leading..];
                    let (field, prefix) = match entry.directives.as_slice() {
                        [] => ("package.files.entries", ""),
                        [FileDirective::Doc] => ("package.files.doc", "%doc"),
                        [FileDirective::License] => ("package.files.license", "%license"),
                        _ => return Err("package.files: unsupported directives".into()),
                    };
                    let path = entry.path.as_ref().ok_or("package.files: missing path")?;
                    let remainder = text
                        .strip_prefix(prefix)
                        .ok_or("package.files: AST/source directive mismatch")?;
                    if !prefix.is_empty() && !remainder.starts_with([' ', '\t']) {
                        return Err("package.files: unsupported directive separator".into());
                    }
                    let start = range.start + leading + prefix.len();
                    if path.path.literal_str().is_none()
                        && remainder.split_whitespace().count() != 1
                    {
                        return Err(format!(
                            "{field}: macro-containing whitespace path lists are unsupported"
                        ));
                    }
                    let mut tokens = Vec::new();
                    let mut offset = 0;
                    for token in remainder.split_whitespace() {
                        valid_path(token, field)?;
                        let relative = remainder[offset..]
                            .find(token)
                            .ok_or("package.files: missing token")?
                            + offset;
                        tokens.push(start + relative..start + relative + token.len());
                        offset = relative + token.len();
                    }
                    if tokens.is_empty() || (prefix.is_empty() && tokens.len() != 1) {
                        return Err(format!("{field}: unsupported path list"));
                    }
                    for token in tokens {
                        self.list_item(field, token, range.clone())?;
                    }
                }
                _ => return Err("package.files: conditional or unknown item is unsupported".into()),
            }
        }
        if self.selection.is_empty() {
            check_coverage(&self.source, body, &mut coverage)?;
        }
        Ok(())
    }

    fn comments(
        &mut self,
        mut comments: Vec<Range<usize>>,
        assets: &[Range<usize>],
    ) -> Result<(), String> {
        comments.sort_by_key(|range| range.start);
        let mut ordinary: Vec<Range<usize>> = Vec::new();
        let mut copyright = Copyright {
            years: Vec::new(),
            holders: List::default(),
        };
        let mut holder_values = Vec::new();
        let mut shared_years: Option<String> = None;
        for range in comments {
            let raw = line(&self.source, &range)?.to_owned();
            let field = comment_field(&raw);
            if !(self.selects(field)
                || (field == "spec.copyright-years" && self.selects("spec.copyright-holders")))
            {
                continue;
            }
            if raw.trim() == "#" {
                continue;
            }
            if assets.contains(&range) {
                continue;
            }
            if raw.contains("RemoteAsset") {
                if !self.selection.is_empty() {
                    continue;
                }
                return Err("sources: malformed, duplicate or orphan RemoteAsset comment".into());
            }
            if let Some(value) = raw.strip_prefix("# SPDX-FileCopyrightText: (C) ") {
                let (years, holder) = value
                    .split_once(' ')
                    .ok_or("spec.copyright-holders: missing holder")?;
                crate::spec_metadata::validate_years(years)?;
                valid_text(holder, "spec.copyright-holders", false)?;
                if let Some(previous) = copyright.holders.lines.last()
                    && previous.end != range.start
                {
                    return Err("spec.copyright-holders: declarations must be contiguous".into());
                }
                if shared_years
                    .as_deref()
                    .is_some_and(|previous| previous != years)
                {
                    return Err("spec.copyright-years: mixed years cannot be mapped".into());
                }
                shared_years = Some(years.to_owned());
                let start = range.start + raw.len() - value.len();
                copyright.years.push(start..start + years.len());
                copyright
                    .holders
                    .items
                    .push(start + years.len() + 1..range.start + raw.len());
                copyright.holders.lines.push(range);
                holder_values.push(Value::String(holder.to_owned()));
            } else if let Some(value) = raw.strip_prefix("# SPDX-FileContributor: ") {
                let value_range = range.start + raw.len() - value.len()..range.start + raw.len();
                self.list_item("spec.contributors", value_range, range)?;
            } else if let Some(value) = raw.strip_prefix(concat!("# SPDX-License-", "Identifier: "))
            {
                self.scalar(
                    "spec.license",
                    range.start + raw.len() - value.len()..range.start + raw.len(),
                    false,
                )?;
            } else {
                if raw.contains("SPDX-FileCopyrightText:")
                    || raw.contains("SPDX-FileContributor:")
                    || raw.contains(concat!("SPDX-License-", "Identifier:"))
                {
                    return Err("spec: unsupported SPDX declaration shape".into());
                }
                valid_comments(&raw)?;
                if let Some(previous) = ordinary
                    .last_mut()
                    .filter(|previous| previous.end == range.start)
                {
                    previous.end = range.end;
                } else {
                    ordinary.push(range);
                }
            }
        }
        if let Some(years) = shared_years {
            if self.selects("spec.copyright-years") {
                insert(
                    &mut self.document,
                    "spec.copyright-years",
                    Value::String(years),
                )?;
            }
            if self.selects("spec.copyright-holders") {
                insert(
                    &mut self.document,
                    "spec.copyright-holders",
                    Value::Array(holder_values),
                )?;
            }
            self.copyright = Some(copyright);
        }
        for range in ordinary {
            let end = range.end - usize::from(self.source[range.clone()].ends_with('\n'));
            self.list_item("spec.comments", range.start..end, range)?;
        }
        Ok(())
    }

    fn replace_list(
        &self,
        list: &List,
        values: &[&str],
        field: &str,
        prefix: &str,
        changes: &mut Vec<(Range<usize>, String)>,
    ) -> Result<(), String> {
        if values.len() == list.items.len() {
            for (range, value) in list.items.iter().zip(values) {
                if *value != &self.source[range.clone()] {
                    changes.push((range.clone(), (*value).to_owned()));
                }
            }
        } else if values.is_empty() {
            for range in &list.lines {
                changes.push((range.clone(), String::new()));
            }
        } else {
            let first = list.lines.first().ok_or_else(|| {
                format!("{field}: adding a previously absent group is unsupported")
            })?;
            let last = list
                .lines
                .last()
                .ok_or_else(|| format!("{field}: missing group"))?;
            if list
                .lines
                .windows(2)
                .any(|pair| pair[0].end != pair[1].start)
            {
                return Err(format!(
                    "{field}: resizing across separate source groups is unsupported"
                ));
            }
            let mut replacement = String::new();
            for value in values {
                replacement.push_str(prefix);
                replacement.push_str(value);
                replacement.push('\n');
            }
            if !self.source[first.start..last.end].ends_with('\n') {
                replacement.pop();
            }
            changes.push((first.start..last.end, replacement));
        }
        Ok(())
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

fn preamble_field(tag: &Tag) -> Option<String> {
    Some(match tag {
        Tag::Name => "package.name".into(),
        Tag::Version => "package.version".into(),
        Tag::Release => "spec.release".into(),
        Tag::Summary => "package.summary".into(),
        Tag::License => "package.license".into(),
        Tag::URL => "package.url".into(),
        Tag::BuildRequires => "build-requires.rpm".into(),
        Tag::Source(number) => format!("sources.{}", number.unwrap_or(0)),
        Tag::Other(name) if name.eq_ignore_ascii_case("BuildSystem") => "build.system".into(),
        _ => return None,
    })
}

fn section_field(section: &Section<Span>) -> Option<&'static str> {
    match section {
        Section::Description { subpkg: None, .. } => Some("package.description"),
        Section::Files { subpkg: None, .. } => Some("package.files"),
        Section::Changelog { .. } => Some("spec.changelog"),
        _ => None,
    }
}

fn comment_field(raw: &str) -> &'static str {
    if raw.contains("RemoteAsset") {
        "sources"
    } else if raw.contains("SPDX-FileCopyrightText:") {
        "spec.copyright-years"
    } else if raw.contains("SPDX-FileContributor:") {
        "spec.contributors"
    } else if raw.contains(concat!("SPDX-License-", "Identifier:")) {
        "spec.license"
    } else {
        "spec.comments"
    }
}

fn reject_selected_conditional(
    source: &str,
    items: &[SpecItem<Span>],
    selection: &[String],
) -> Result<(), String> {
    for item in items {
        let field = match item {
            SpecItem::Preamble(item) => preamble_field(&item.tag),
            SpecItem::Section(section) => section_field(section).map(str::to_owned),
            SpecItem::Comment(comment) => {
                let range = checked_range(source, comment.data)?;
                let field = comment_field(line(source, &range)?);
                if field == "spec.copyright-years" && selected(selection, "spec.copyright-holders")
                {
                    return Err("spec.copyright-holders: conditional selection is ambiguous".into());
                }
                Some(field.into())
            }
            SpecItem::Conditional(conditional) => {
                for branch in &conditional.branches {
                    reject_selected_conditional(source, &branch.body, selection)?;
                }
                if let Some(items) = &conditional.otherwise {
                    reject_selected_conditional(source, items, selection)?;
                }
                None
            }
            _ => None,
        };
        if let Some(field) = field.filter(|field| selected(selection, field)) {
            return Err(format!("{field}: conditional selection is ambiguous"));
        }
    }
    Ok(())
}

fn checked_range(source: &str, span: Span) -> Result<Range<usize>, String> {
    let range = span.start_byte..span.end_byte;
    source
        .get(range.clone())
        .ok_or("source: invalid AST span")?;
    Ok(range)
}

fn line<'a>(source: &'a str, range: &Range<usize>) -> Result<&'a str, String> {
    let raw = source
        .get(range.clone())
        .ok_or("source: invalid line span")?;
    let raw = raw.strip_suffix('\n').unwrap_or(raw);
    if raw.contains('\n') {
        return Err("source: multiline preamble, comment or file entry is unsupported".into());
    }
    Ok(raw)
}

fn check_coverage(
    source: &str,
    whole: Range<usize>,
    ranges: &mut [Range<usize>],
) -> Result<(), String> {
    ranges.sort_by_key(|range| range.start);
    let mut offset = whole.start;
    for range in ranges {
        if range.start < offset
            || range.end > whole.end
            || !source[offset..range.start].trim().is_empty()
        {
            return Err("source: unmapped content or overlapping AST spans".into());
        }
        offset = range.end;
    }
    if !source[offset..whole.end].trim().is_empty() {
        return Err("source: unmapped trailing content".into());
    }
    Ok(())
}

fn insert(table: &mut Table, field: &str, value: Value) -> Result<(), String> {
    if let Some((head, tail)) = field.split_once('.') {
        let child = table
            .entry(head.to_owned())
            .or_insert_with(|| Value::Table(Table::new()));
        let child = child
            .as_table_mut()
            .ok_or_else(|| format!("{field}: conflicting field"))?;
        insert(child, tail, value)
    } else if table.insert(field.to_owned(), value).is_some() {
        Err(format!("{field}: duplicate scalar field"))
    } else {
        Ok(())
    }
}

fn string<'a>(table: &'a Table, field: &str) -> Result<&'a str, String> {
    lookup(table, field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field}: required string field"))
}

fn strings<'a>(table: &'a Table, field: &str) -> Result<Vec<&'a str>, String> {
    lookup(table, field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field}: required array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("{field}: expected string array"))
        })
        .collect()
}

fn valid_text(value: &str, field: &str, multiline: bool) -> Result<(), String> {
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

/// Retains raw main-package context even when an edit projects only Source fields.
/// Missing and repeated fields are unavailable, not guessed from an arbitrary tag.
fn source_fields(source: &str, parsed: &ParseResult<Span>) -> BTreeMap<String, String> {
    fn collect(
        source: &str,
        items: &[SpecItem<Span>],
        conditional: bool,
        fields: &mut BTreeMap<String, String>,
        unavailable: &mut Vec<String>,
    ) {
        for item in items {
            let item = match item {
                SpecItem::Preamble(item) => item,
                SpecItem::Conditional(condition) => {
                    for branch in &condition.branches {
                        collect(source, &branch.body, true, fields, unavailable);
                    }
                    if let Some(items) = &condition.otherwise {
                        collect(source, items, true, fields, unavailable);
                    }
                    continue;
                }
                SpecItem::MacroDef(definition) => {
                    unavailable.push(definition.name.clone());
                    continue;
                }
                SpecItem::Include(_) | SpecItem::Statement(_) => {
                    unavailable.extend(["name", "version", "url"].map(str::to_owned));
                    continue;
                }
                _ => continue,
            };
            let name = match item.tag {
                Tag::Name => "name",
                Tag::Version => "version",
                Tag::URL => "url",
                _ => continue,
            };
            if conditional || !item.qualifiers.is_empty() || item.lang.is_some() {
                unavailable.push(name.to_owned());
                continue;
            }
            let Some(raw) = source.get(item.data.start_byte..item.data.end_byte) else {
                continue;
            };
            let Some((_, value)) = raw.split_once(':') else {
                continue;
            };
            if fields
                .insert(name.to_owned(), value.trim().to_owned())
                .is_some()
            {
                unavailable.push(name.to_owned());
            }
        }
    }
    let mut fields = BTreeMap::new();
    let mut unavailable = Vec::new();
    collect(
        source,
        &parsed.spec.items,
        false,
        &mut fields,
        &mut unavailable,
    );
    for name in unavailable {
        fields.remove(&name);
    }
    fields
}

fn valid_source_url(
    value: &str,
    field: &str,
    context: &BTreeMap<String, String>,
) -> Result<(), String> {
    let fields = context
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    crate::source::validate_expression(value, &fields)
        .map(|_| ())
        .map_err(|reason| format!("{field}: {reason}"))
}

fn valid_hash(value: &str, field: &str) -> Result<(), String> {
    crate::source::validate_sha256(value).map_err(|reason| format!("{field}: {reason}"))
}

fn valid_path(value: &str, field: &str) -> Result<(), String> {
    valid_text(value, field, false)?;
    if value.chars().any(char::is_whitespace) || value.contains(['\'', '"', '\\', '#']) {
        return Err(format!(
            "{field}: quoted, escaped, whitespace or comment-bearing paths are unsupported"
        ));
    }
    Ok(())
}

fn valid_comments(value: &str) -> Result<(), String> {
    valid_text(value, "spec.comments", true)?;
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
