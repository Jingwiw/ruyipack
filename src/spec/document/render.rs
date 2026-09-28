// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Validate edited values and assemble nonoverlapping source replacements.

use std::ops::Range;
use toml::Table;

use super::table::{string, strings, validate_shape};
use super::{List, Snapshot, validate_comments, validate_file_path, validate_text};

impl Snapshot<'_> {
    pub(crate) fn render(&self, edited: &Table, defines: &[String]) -> Result<String, String> {
        self.render_changes(edited, true, defines)
    }

    /// Keep original digest markers until their replacement bytes are known.
    pub(crate) fn render_before_hashing(
        &self,
        edited: &Table,
        defines: &[String],
    ) -> Result<String, String> {
        for (field, range) in &self.digest_markers {
            if self.source[range.clone()].contains('%') {
                return Err(format!(
                    "{field}: macro-bearing RemoteAsset cannot be hashed safely; repair the marker first"
                ));
            }
        }
        self.render_changes(edited, false, defines)
    }

    fn render_changes(
        &self,
        edited: &Table,
        with_digests: bool,
        defines: &[String],
    ) -> Result<String, String> {
        validate_shape(&self.document, edited)?;
        let mut changes = Vec::new();
        for scalar in &self.scalars {
            let value = string(edited, &scalar.field)?;
            validate_text(value, &scalar.field, scalar.multiline)?;
            if scalar.field == "package.url" {
                crate::source::reject_credentials(value)
                    .map_err(|reason| format!("package.url: {reason}"))?;
            }
            if value != &self.source[scalar.range.clone()] {
                changes.push((scalar.range.clone(), value.to_owned()));
            }
        }
        for (field, range) in self.digest_markers.iter().filter(|_| with_digests) {
            let value = string(edited, field)?;
            let profile = crate::profile::load();
            // Empty represents an already-bare marker, never a made-up digest.
            // Existing digests cannot be erased by accidentally clearing the draft.
            if value.is_empty()
                && self.source[range.clone()].trim_end() == profile.remote_asset_bare
            {
                continue;
            }
            validate_sha256(value, field)?;
            if value != string(&self.document, field)? {
                changes.push((
                    range.clone(),
                    format!("{}\n", profile.remote_asset(Some(value))),
                ));
            }
        }
        for (field, list) in &self.lists {
            let values = strings(edited, field)?;
            if field == "spec.comments" && !values.is_empty() && values.len() != list.items.len() {
                return Err("spec.comments: edit lines within an existing comment block; adding or regrouping blocks is unsupported".into());
            }
            for value in &values {
                if field == "spec.comments" {
                    validate_comments(value)?;
                } else if field.starts_with("package.files.") {
                    validate_file_path(value, field)?;
                } else {
                    validate_text(value, field, false)?;
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
                validate_text(holder, "spec.copyright-holders", false)?;
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
            .array_windows::<2>()
            .any(|[(left, _), (right, _)]| left.end > right.start)
        {
            return Err("edit: overlapping replacements".into());
        }
        let mut output = String::with_capacity(self.source.len());
        let mut cursor = 0;
        for (range, value) in changes {
            output.push_str(&self.source[cursor..range.start]);
            output.push_str(&value);
            cursor = range.end;
        }
        output.push_str(&self.source[cursor..]);
        // Resolve selected URL expressions against the actual candidate, not a second
        // cached projection of package fields. Literal repairs need no macro context.
        let mut sources = None;
        for scalar in &self.scalars {
            if let Some(number) = scalar.field.strip_circumfix("sources.", ".url") {
                let value = string(edited, &scalar.field)?;
                let resolved = crate::spec::expression::substitute_fields(value, &[])
                    .or_else(|_| {
                        let sources = sources.get_or_insert_with(|| {
                            crate::spec::sources::resolve(
                                &crate::spec::ParsedSpec::parse(&output),
                                defines,
                            )
                        });
                        let source = sources
                            .as_ref()
                            .map_err(Clone::clone)?
                            .sources
                            .get(&number.parse::<u32>().expect("captured Source number"))
                            .ok_or("selected Source is absent from the candidate")?;
                        source.url.clone()
                    })
                    .map_err(|reason| format!("{}: {reason}", scalar.field))?;
                crate::source::validate_authoring_url(&resolved)
                    .map_err(|reason| format!("{}: {reason}", scalar.field))?;
            }
        }
        Ok(output)
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
        } else if field == "build-requires.rpm"
            && values.len() > list.items.len()
            && list
                .items
                .iter()
                .zip(values)
                .all(|(range, value)| self.source[range.clone()] == **value)
            && let Some(last) = list.lines.last()
        {
            // Append only: leave existing dependency groups and their comments intact.
            // Other regroupings remain ambiguous and use the contiguous-block rule.
            let mut added = String::new();
            if !self.source[last.clone()].ends_with('\n') {
                added.push('\n');
            }
            for value in &values[list.items.len()..] {
                added.push_str(prefix);
                added.push_str(value);
                added.push('\n');
            }
            changes.push((last.end..last.end, added));
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
                .array_windows::<2>()
                .any(|[left, right]| left.end != right.start)
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

fn validate_sha256(value: &str, field: &str) -> Result<(), String> {
    crate::source::validate_sha256(value).map_err(|reason| format!("{field}: {reason}"))
}
