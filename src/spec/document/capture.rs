// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Locate selected fields without requiring old values to be valid replacements.

use rpm_spec::{
    ast::{FileDirective, FilesContent, Section, Span, SpecItem, Tag},
    parse_result::Severity,
};
use std::{collections::BTreeMap, ops::Range};
use toml::{Table, Value};

use super::table::{insert, lookup, lookup_mut};
use super::{
    Copyright, List, Scalar, Snapshot, selected, validate_comments, validate_file_path,
    validate_text,
};
use crate::spec::ParsedSpec;

impl<'src> Snapshot<'src> {
    pub(crate) fn capture_selected(
        spec: &ParsedSpec<'src>,
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
        let needs_sources = selected(selection, "sources");
        let mut snapshot = Self {
            source: source.into(),
            document: Table::new(),
            selection: selection.to_vec(),
            scalars: Vec::new(),
            digest_markers: BTreeMap::new(),
            lists: BTreeMap::new(),
            copyright: None,
        };
        snapshot.list("spec.contributors", "# SPDX-FileContributor: ");
        snapshot.list("spec.comments", "");
        snapshot.list("build-requires.rpm", "BuildRequires:  ");
        let profile = crate::profile::load();
        let mut coverage = Vec::new();
        let mut comments = Vec::new();
        let mut consumed_assets = Vec::new();
        let mut sections = Vec::new();
        // Editing locates declarations; static resolution supplies implicit identities.
        // Explicit fields remain repairable even when other Source values are unknown.
        let source_numbers = needs_sources
            .then(|| crate::spec::sources::resolve(spec, &[]))
            .and_then(Result::ok)
            .map(|sources| {
                sources
                    .sources
                    .into_iter()
                    .map(|(number, source)| (source.span.bytes.start, number))
                    .collect::<BTreeMap<_, _>>()
            });
        for (index, item) in parsed.spec.items.iter().enumerate() {
            match item {
                SpecItem::Blank => {}
                SpecItem::Comment(comment) => {
                    let range = checked_range(source, comment.data)?;
                    coverage.push(range.clone());
                    comments.push(range);
                }
                SpecItem::Preamble(item) => {
                    let number = if let Tag::Source(explicit) = item.tag
                        && needs_sources
                    {
                        Some(explicit.or_else(|| source_numbers.as_ref()
                            .and_then(|numbers| numbers.get(&item.data.start_byte).copied()))
                            .ok_or("sources: implicit Source number is uncertain after unsupported or conditional declarations")?)
                    } else {
                        None
                    };
                    let field = number
                        .map(|number| format!("sources.{number}"))
                        .or_else(|| preamble_field(&item.tag));
                    if !selection.is_empty() && !field.is_some_and(|field| snapshot.selects(&field))
                    {
                        continue;
                    }
                    let range = checked_range(source, item.data)?;
                    coverage.push(range.clone());
                    let raw = line(source, &range)?;
                    let (name, value) = raw.split_once(':').ok_or("preamble: missing colon")?;
                    if !item.qualifiers.is_empty() || item.lang.is_some() {
                        return Err(format!(
                            "preamble {name:?}: qualified or localized tag cannot be edited; select a supported field such as --field package.version"
                        ));
                    }
                    let value_start = range.start + name.len() + 1 + value.len()
                        - value.trim_start_matches([' ', '\t']).len();
                    let value_end = range.start + raw.trim_end_matches([' ', '\t']).len();
                    let value_range = value_start..value_end;
                    let (field, expected) = match &item.tag {
                        Tag::BuildRequires => {
                            if !name.trim().eq_ignore_ascii_case("BuildRequires") {
                                return Err("build-requires.rpm: AST/source header mismatch".into());
                            }
                            snapshot.list_item("build-requires.rpm", value_range, range)?;
                            continue;
                        }
                        Tag::Source(_) => {
                            let number = number.ok_or("sources: unresolved Source number")?;
                            let identity = format!("sources.{number}");
                            if lookup(&snapshot.document, &format!("{identity}.url")).is_some() {
                                return Err(format!("{identity}: duplicate Source identity"));
                            }
                            let expected = match &item.tag {
                                Tag::Source(Some(number)) => format!("Source{number}"),
                                _ => "Source".to_owned(),
                            };
                            let missing_asset = || {
                                format!(
                                    "{identity} ({name}): no adjacent RemoteAsset marker; local or unmarked Sources are not editable; select an individual marked Source with --field sources.N"
                                )
                            };
                            // RemoteAsset belongs to the immediately following Source.
                            // Requiring byte adjacency avoids stealing a different asset's
                            // digest across blank lines or unrelated comments.
                            let Some(SpecItem::Comment(previous)) =
                                index.checked_sub(1).and_then(|i| parsed.spec.items.get(i))
                            else {
                                return Err(missing_asset());
                            };
                            let asset = checked_range(source, previous.data)?;
                            if asset.end != range.start {
                                return Err(format!(
                                    "{identity}.sha256: RemoteAsset must be immediately adjacent"
                                ));
                            }
                            let asset_text = &source[asset.clone()];
                            if !asset_text.starts_with(profile.remote_asset_bare.as_str()) {
                                return Err(missing_asset());
                            }
                            let hash = if asset_text.strip_suffix('\n')
                                == Some(profile.remote_asset_bare.as_str())
                            {
                                ""
                            } else {
                                // Map damaged values too, so the selected digest can be repaired.
                                asset_text
                                    .strip_circumfix(profile.remote_asset_prefix.as_str(), '\n')
                                    .ok_or_else(|| {
                                        format!("{identity}.sha256: unsupported RemoteAsset syntax")
                                    })?
                            };
                            let field = format!("{identity}.sha256");
                            if snapshot.selects(&field) {
                                insert(
                                    &mut snapshot.document,
                                    &field,
                                    Value::String(hash.to_owned()),
                                )?;
                                snapshot.digest_markers.insert(field, asset.clone());
                            }
                            consumed_assets.push(asset);
                            (format!("{identity}.url"), expected)
                        }
                        _ => {
                            let (field, expected) =
                                scalar_preamble(&item.tag).ok_or_else(|| {
                                    format!("preamble: unsupported tag {:?}", item.tag)
                                })?;
                            (field.to_owned(), expected.to_owned())
                        }
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
                        Section::Files { subpkg: None, file_lists, data, .. } => {
                            if !file_lists.is_empty() {
                                let range = checked_range(source, *data)?;
                                let header = source[range].lines().next().unwrap_or("%files");
                                return Err(format!("package.files: external file list in {header:?} is not editable; select another field such as --field package.version"));
                            }
                            ("files", *data)
                        },
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
            validate_coverage(source, 0..source.len(), &mut coverage)?;
        }
        snapshot.comments(comments, &consumed_assets)?;
        if selection.is_empty() && lookup(&snapshot.document, "package.name").is_none() {
            return Err("package.name: required main package is missing".into());
        }
        Ok(snapshot)
    }

    fn scalar(&mut self, field: &str, range: Range<usize>, multiline: bool) -> Result<(), String> {
        if !self.selects(field) {
            return Ok(());
        }
        let value = self
            .source
            .get(range.clone())
            .ok_or_else(|| format!("{field}: invalid value span"))?;
        validate_text(value, field, multiline)?;
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
        validate_text(value, field, field == "spec.comments")?;
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
                    let text = raw.trim_start_matches([' ', '\t']);
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
                    if path.path.literal_str().is_none()
                        && remainder.split_whitespace().count() != 1
                    {
                        return Err(format!(
                            "{field}: macro-containing whitespace path lists are unsupported"
                        ));
                    }
                    let mut tokens = Vec::new();
                    for token in remainder.split_whitespace() {
                        validate_file_path(token, field)?;
                        tokens.push(
                            self.source
                                .substr_range(token)
                                .expect("split tokens borrow the source")
                                .into(),
                        );
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
            validate_coverage(&self.source, body, &mut coverage)?;
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
            let raw = line(&self.source, &range)?;
            let field = comment_field(raw);
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
            } else if let Some(value) = crate::spec_metadata::license_declaration(raw) {
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
                validate_comments(raw)?;
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
}

fn scalar_preamble(tag: &Tag) -> Option<(&'static str, &'static str)> {
    Some(match tag {
        Tag::Name => ("package.name", "Name"),
        Tag::Version => ("package.version", "Version"),
        Tag::Release => ("spec.release", "Release"),
        Tag::Summary => ("package.summary", "Summary"),
        Tag::License => ("package.license", "License"),
        Tag::URL => ("package.url", "URL"),
        Tag::Other(name) if name.eq_ignore_ascii_case("BuildSystem") => {
            ("build.system", "BuildSystem")
        }
        _ => return None,
    })
}

fn preamble_field(tag: &Tag) -> Option<String> {
    Some(match tag {
        Tag::BuildRequires => "build-requires.rpm".into(),
        Tag::Source(Some(number)) => format!("sources.{number}"),
        Tag::Source(None) => "sources".into(),
        _ => scalar_preamble(tag)?.0.into(),
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

fn validate_coverage(
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
