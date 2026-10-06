// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Source-bound configure fragments; absent slots do not override RPM defaults.

use super::{Snapshot, table, validate_text};
use crate::spec::ParsedSpec;
use rpm_spec::ast::{BuildScriptKind, BuildScriptPlacement, Section, Span, SpecItem};
use std::ops::Range;
use toml::{Table, Value};

pub(super) struct Script {
    field: String,
    header: &'static str,
    range: Range<usize>,
    present: bool,
}

pub(super) fn is_conf(section: &Section<Span>) -> bool {
    matches!(
        section,
        Section::BuildScript {
            kind: BuildScriptKind::Conf,
            ..
        }
    )
}

pub(super) fn span(section: &Section<Span>) -> Span {
    match section {
        Section::BuildScript { data, .. } => *data,
        _ => unreachable!("configure section"),
    }
}

impl Snapshot<'_> {
    pub(super) fn capture_scripts(&mut self, spec: &ParsedSpec<'_>) -> Result<(), String> {
        if !self.selects("build.stages.conf") {
            return Ok(());
        }
        if spec
            .parsed
            .spec
            .items
            .iter()
            .any(|item| matches!(item, SpecItem::Include(_) | SpecItem::Statement(_)))
        {
            return Err("conf: include or generated sections prevent a complete mapping".into());
        }
        // Insert before the first top-level build/output section, not inside a
        // script, conditional branch, or changelog body.
        let anchor = spec.parsed.spec.items.iter().find_map(|item| match item {
            SpecItem::Section(section) => match section.as_ref() {
                Section::BuildScript { data, .. }
                | Section::Files { data, .. }
                | Section::Changelog { data, .. } => Some(data.start_byte),
                _ => None,
            },
            _ => None,
        });
        for (mode, placement, header) in [
            ("prepend", BuildScriptPlacement::Prepend, "%conf -p"),
            ("replace", BuildScriptPlacement::Main, "%conf"),
            ("append", BuildScriptPlacement::Append, "%conf -a"),
        ] {
            let field = format!("build.stages.conf.{mode}");
            if !self.selects(&field) {
                continue;
            }
            let mut found = None;
            for item in &spec.parsed.spec.items {
                if let SpecItem::Section(section) = item
                    && let Section::BuildScript {
                        kind: BuildScriptKind::Conf,
                        placement: actual,
                        data,
                        ..
                    } = section.as_ref()
                    && *actual == placement
                    && found.replace(*data).is_some()
                {
                    return Err(format!("{field}: duplicate section"));
                }
            }
            let (range, value) = if let Some(data) = found {
                let range = data.start_byte..data.end_byte;
                let raw = self
                    .source
                    .get(range.clone())
                    .ok_or("conf: invalid source span")?;
                let (line, body) = raw.split_once('\n').unwrap_or((raw, ""));
                if line.trim_end_matches('\r') != header {
                    return Err(format!("{field}: unsupported section header"));
                }
                (
                    range,
                    super::logical_text(body).trim_end_matches('\n').to_owned(),
                )
            } else {
                let Some(offset) = anchor else {
                    continue;
                };
                (offset..offset, String::new())
            };
            table::insert(&mut self.document, &field, Value::String(value))?;
            self.scripts.push(Script {
                field,
                header,
                range,
                present: found.is_some(),
            });
        }
        Ok(())
    }

    pub(super) fn replace_scripts(
        &self,
        edited: &Table,
        changes: &mut Vec<(Range<usize>, String)>,
    ) -> Result<(), String> {
        for script in &self.scripts {
            let value = table::string(edited, &script.field)?;
            if value == table::string(&self.document, &script.field)? {
                continue;
            }
            validate_text(value, &script.field, true)?;
            // Clearing an existing replacement keeps an explicit empty %conf;
            // it must not silently restore the BuildSystem's default configure.
            let text = if value.is_empty() && (!script.present || script.header != "%conf") {
                String::new()
            } else {
                format!("{}\n{value}\n\n", script.header)
            };
            changes.push((script.range.clone(), text));
        }
        Ok(())
    }
}
