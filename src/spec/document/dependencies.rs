// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Edit declarations in their lexical branch, never evaluate or flatten RPM conditions.

use super::{Snapshot, table};
use crate::{dependency, spec::ParsedSpec};
use rpm_spec::ast::{PreambleContent, PreambleItem, Section, Span, SpecItem, Tag};
use std::{collections::BTreeMap, ops::Range};
use toml::{Table, Value};

pub(super) struct Group {
    pub field: String,
    pub label: String,
    anchor: usize,
    lines: Vec<Range<usize>>,
    values: Vec<String>,
}

struct Scope<'a> {
    key: String,
    label: String,
    anchor: usize,
    items: Vec<&'a PreambleItem<Span>>,
}

fn header(source: &str, start: usize) -> Result<(&str, usize), String> {
    let text = source.get(start..).ok_or("invalid dependency scope span")?;
    let len = text.find('\n').map_or(text.len(), |n| n + 1);
    Ok((text[..len].trim_end(), start + len))
}

fn collect<'a>(
    items: &'a [SpecItem<Span>],
    source: &str,
    scope: &mut Scope<'a>,
    result: &mut Vec<Scope<'a>>,
) -> Result<(), String> {
    let mut condition = 0;
    let mut package = 0;
    for item in items {
        match item {
            SpecItem::Preamble(p) if p.tag == Tag::BuildRequires => scope.items.push(p),
            SpecItem::Conditional(c) => {
                condition += 1;
                for (index, branch) in c.branches.iter().enumerate() {
                    let (label, anchor) = header(source, branch.data.start_byte)?;
                    let mut child = Scope {
                        key: format!("{}if{condition}-then{}-", scope.key, index + 1),
                        label: format!("{} / {label}", scope.label),
                        anchor,
                        items: vec![],
                    };
                    collect(&branch.body, source, &mut child, result)?;
                    result.push(child);
                }
                if let Some(body) = &c.otherwise {
                    let mut child = Scope {
                        key: format!("{}if{condition}-else-", scope.key),
                        label: format!("{} / condition {condition}: else", scope.label),
                        anchor: header(
                            source,
                            c.branches
                                .last()
                                .ok_or("else without condition")?
                                .data
                                .end_byte,
                        )?
                        .1,
                        items: vec![],
                    };
                    collect(body, source, &mut child, result)?;
                    result.push(child);
                }
            }
            SpecItem::Section(section) => {
                if let Section::Package { content, data, .. } = section.as_ref() {
                    package += 1;
                    let (label, anchor) = header(source, data.start_byte)?;
                    let mut child = Scope {
                        key: format!("{}package{package}-", scope.key),
                        label: format!("{} / {label}", scope.label),
                        anchor,
                        items: vec![],
                    };
                    collect_preamble(content, source, &mut child, result)?;
                    result.push(child);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn collect_preamble<'a>(
    items: &'a [PreambleContent<Span>],
    source: &str,
    scope: &mut Scope<'a>,
    result: &mut Vec<Scope<'a>>,
) -> Result<(), String> {
    let mut condition = 0;
    for item in items {
        match item {
            PreambleContent::Item(p) if p.tag == Tag::BuildRequires => scope.items.push(p),
            PreambleContent::Conditional(c) => {
                condition += 1;
                for (index, branch) in c.branches.iter().enumerate() {
                    let (label, anchor) = header(source, branch.data.start_byte)?;
                    let mut child = Scope {
                        key: format!("{}if{condition}-then{}-", scope.key, index + 1),
                        label: format!("{} / {label}", scope.label),
                        anchor,
                        items: vec![],
                    };
                    collect_preamble(&branch.body, source, &mut child, result)?;
                    result.push(child);
                }
                if let Some(body) = &c.otherwise {
                    let mut child = Scope {
                        key: format!("{}if{condition}-else-", scope.key),
                        label: format!("{} / condition {condition}: else", scope.label),
                        anchor: header(
                            source,
                            c.branches
                                .last()
                                .ok_or("else without condition")?
                                .data
                                .end_byte,
                        )?
                        .1,
                        items: vec![],
                    };
                    collect_preamble(body, source, &mut child, result)?;
                    result.push(child);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

impl Snapshot<'_> {
    pub(super) fn capture_dependencies(&mut self, spec: &ParsedSpec<'_>) -> Result<(), String> {
        if !self.selects("build-requires") {
            return Ok(());
        }
        let mut root = Scope {
            key: String::new(),
            label: "Unconditional".into(),
            anchor: spec
                .parsed
                .spec
                .items
                .iter()
                .take_while(|item| !matches!(item, SpecItem::Section(_)))
                .filter_map(|item| match item {
                    SpecItem::Preamble(item) => Some(item.data.end_byte),
                    _ => None,
                })
                .last()
                .unwrap_or(0),
            items: vec![],
        };
        let mut scopes = vec![];
        collect(
            &spec.parsed.spec.items,
            spec.source(),
            &mut root,
            &mut scopes,
        )?;
        if let Some(first) = root.items.first() {
            root.anchor = first.data.start_byte;
        }
        scopes.insert(0, root);
        for scope in scopes {
            let prefix = if scope.key.is_empty() {
                "build-requires".to_owned()
            } else {
                format!("build-requires.{}", scope.key.trim_end_matches('-'))
            };
            let mut groups = BTreeMap::new();
            for &namespace in dependency::NAMESPACES {
                let field = format!("{prefix}.{namespace}");
                if self.selects(&field) {
                    groups.insert(
                        namespace,
                        Group {
                            field,
                            label: scope.label.clone(),
                            anchor: scope.anchor,
                            lines: vec![],
                            values: vec![],
                        },
                    );
                }
            }
            if groups.is_empty() {
                continue;
            }
            for item in scope.items {
                let range = item.data.start_byte..item.data.end_byte;
                let raw = spec
                    .source()
                    .get(range.clone())
                    .ok_or("invalid dependency span")?;
                let (tag, value) = raw
                    .trim_end()
                    .split_once(':')
                    .ok_or("invalid dependency declaration")?;
                if !tag.eq_ignore_ascii_case("BuildRequires") || !item.qualifiers.is_empty() {
                    return Err("qualified BuildRequires is not editable".into());
                }
                let (namespace, value) = dependency::split(value.trim());
                if let Some(group) = groups.get_mut(namespace) {
                    group.lines.push(range);
                    group.values.push(value);
                }
            }
            for (namespace, group) in groups {
                // Empty groups are offered when explicitly selected, without filling every view with empty tables.
                if group.lines.is_empty()
                    && !(scope.key.is_empty() && namespace == "rpm")
                    && !self.selection.iter().any(|f| f == &group.field)
                {
                    continue;
                }
                table::insert(
                    &mut self.document,
                    &group.field,
                    Value::Array(group.values.iter().cloned().map(Value::String).collect()),
                )?;
                self.dependencies.push(group);
            }
        }
        Ok(())
    }

    pub(crate) fn dependency_scopes(&self) -> Vec<(&str, &str)> {
        let mut scopes = BTreeMap::new();
        for group in &self.dependencies {
            scopes.insert(
                group.field.rsplit_once('.').expect("dependency path").0,
                group.label.as_str(),
            );
        }
        scopes.into_iter().collect()
    }

    pub(super) fn replace_dependencies(
        &self,
        edited: &Table,
        changes: &mut Vec<(Range<usize>, String)>,
    ) -> Result<(), String> {
        for group in &self.dependencies {
            let values = table::strings(edited, &group.field)?;
            let old: Vec<&str> = group.values.iter().map(String::as_str).collect();
            let namespace = group
                .field
                .rsplit('.')
                .next()
                .expect("dependency namespace");
            let expanded = values
                .iter()
                .map(|value| {
                    super::validate_text(value, &group.field, false)?;
                    dependency::expand(namespace, value)
                })
                .collect::<Result<Vec<_>, _>>()?;
            for op in similar::capture_diff_slices(similar::Algorithm::Myers, &old, &values) {
                if op.tag() == similar::DiffTag::Equal {
                    continue;
                }
                let removed = op.old_range();
                let added = op.new_range();
                if removed.len() == added.len() {
                    for (line, value) in group.lines[removed].iter().zip(&expanded[added]) {
                        let raw = &self.source[line.clone()];
                        let colon = raw.find(':').ok_or("dependency declaration lost colon")? + 1;
                        let start = colon + raw[colon..].len()
                            - raw[colon..].trim_start_matches([' ', '\t']).len();
                        changes.push((
                            line.start + start..line.start + raw.trim_end().len(),
                            value.clone(),
                        ));
                    }
                    continue;
                }
                let position = group.lines.get(removed.start).map_or_else(
                    || group.lines.last().map_or(group.anchor, |line| line.end),
                    |line| line.start,
                );
                let mut replacement = String::new();
                if position > 0 && !self.source[..position].ends_with('\n') {
                    replacement.push('\n');
                }
                for value in &expanded[op.new_range()] {
                    replacement.push_str("BuildRequires:  ");
                    replacement.push_str(value);
                    replacement.push('\n');
                }
                // Remove each declaration separately, leaving intervening comments and other namespaces intact.
                if removed.is_empty() {
                    changes.push((position..position, replacement));
                } else {
                    for (index, line) in group.lines[removed].iter().enumerate() {
                        changes.push((
                            line.clone(),
                            if index == 0 {
                                std::mem::take(&mut replacement)
                            } else {
                                String::new()
                            },
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
