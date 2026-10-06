// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Typed CLI and inline assignments to existing source-mapped draft fields.

use std::collections::BTreeSet;

use serde::Deserialize;
use toml::{Table, Value};

use crate::spec::document::table::{lookup, lookup_mut};

/// Strings remain literal; string arrays use TOML value syntax in CLI and inline edits.
pub(super) fn assign(
    document: &mut Table,
    assignments: &[(String, String)],
    additions: &[(String, String)],
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for (field, value) in assignments {
        if !seen.insert(field) {
            return Err(format!("{field}: repeated assignment"));
        }
        let target =
            lookup_mut(document, field).ok_or_else(|| format!("{field}: unknown field"))?;
        *target = replacement(target, value, field)?;
    }
    for (field, value) in additions {
        if seen.contains(field) {
            return Err(format!("{field}: choose either --set or --add, not both"));
        }
        let target =
            lookup_mut(document, field).ok_or_else(|| format!("{field}: unknown field"))?;
        let Value::String(text) = target else {
            return Err(format!("{field}: --add requires a string field"));
        };
        if value.is_empty() {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(value);
    }
    Ok(())
}

fn replacement(current: &Value, text: &str, field: &str) -> Result<Value, String> {
    match current {
        Value::String(_) => Ok(Value::String(text.to_owned())),
        Value::Array(_) => {
            let values = toml::de::ValueDeserializer::parse(text)
                .and_then(Vec::<String>::deserialize)
                .map_err(|error| format!("{field}: expected TOML string array: {error}"))?;
            Ok(Value::Array(
                values.into_iter().map(Value::String).collect(),
            ))
        }
        _ => Err(format!(
            "{field}: direct assignment requires a string or string-array leaf"
        )),
    }
}

/// Edit leaves inline, preserving strings and string-array types.
pub(super) fn edit_inline(document: &mut Table, selection: &[String]) -> Result<(), String> {
    fn leaves(value: &Value, field: &str, fields: &mut Vec<String>) {
        if let Value::Table(table) = value {
            for (key, value) in table {
                leaves(value, &format!("{field}.{key}"), fields);
            }
        } else {
            fields.push(field.to_owned());
        }
    }
    let mut fields = Vec::new();
    for field in selection {
        let value = lookup(document, field).ok_or_else(|| format!("{field}: unknown field"))?;
        leaves(value, field, &mut fields);
    }
    for field in fields {
        let current = lookup(document, &field).expect("selected leaf exists");
        let initial = match current {
            Value::String(value) => value.clone(),
            Value::Array(values) => {
                if values.iter().any(|value| !value.is_str()) {
                    return Err(format!("{field}: expected a string array"));
                }
                current.to_string()
            }
            _ => return Err(format!("{field}: unsupported inline type")),
        };
        let edited = if field == "build.system" {
            let systems = crate::profile::buildsystems::systems().collect::<Vec<_>>();
            crate::prompt::choose(&field, &systems, Some(&initial))
        } else {
            crate::prompt::value(&field, &initial)
        }
        .map_err(|error| format!("{field}: {error}"))?;
        let replacement = replacement(current, &edited, &field)?;
        *lookup_mut(document, &field).expect("selected leaf exists") = replacement;
    }
    Ok(())
}

/// Overlay facts present in authoring without importing its unrelated recipe structure.
pub(super) fn overlay(selected: &mut Table, authoring: &Table) {
    for (key, target) in selected {
        let Some(value) = authoring.get(key) else {
            continue;
        };
        match (target, value) {
            (Value::Table(target), Value::Table(value)) => overlay(target, value),
            (Value::String(target), Value::String(value))
                if target.trim_end_matches('\n') == value.trim_end_matches('\n') => {}
            (target, value) => *target = value.clone(),
        }
    }
}

/// Apply only changed leaves; untouched defaults must not become new author facts.
pub(super) fn update_authoring(
    authoring: &mut Table,
    before: &Table,
    after: &Table,
) -> Result<(), String> {
    for (key, value) in after {
        if before.get(key) == Some(value) {
            continue;
        }
        if let (Some(Value::Table(old)), Value::Table(new)) = (before.get(key), value) {
            let table = authoring
                .entry(key.clone())
                .or_insert_with(|| Value::Table(Table::new()))
                .as_table_mut()
                .ok_or_else(|| format!("{key}: expected a table"))?;
            update_authoring(table, old, new)?;
        } else {
            authoring.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}
