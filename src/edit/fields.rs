// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Typed CLI and inline assignments to existing source-mapped stage fields.

use std::collections::BTreeSet;

use serde::Deserialize;
use toml::{Table, Value};

use crate::spec::document::table::{lookup, lookup_mut};

/// Strings remain literal; string arrays use TOML value syntax in CLI and inline edits.
pub(super) fn assign(document: &mut Table, assignments: &[(String, String)]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for (field, value) in assignments {
        if !seen.insert(field) {
            return Err(format!("{field}: repeated assignment"));
        }
        let target =
            lookup_mut(document, field).ok_or_else(|| format!("{field}: unknown field"))?;
        *target = replacement(target, value, field)?;
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
            Value::Array(_) => current.to_string(),
            _ => return Err(format!("{field}: unsupported inline type")),
        };
        let edited: String = dialoguer::Input::new()
            .with_prompt(&field)
            .with_initial_text(&initial)
            .allow_empty(true)
            .interact_text()
            .map_err(|error| format!("{field}: {error}"))?;
        let replacement = replacement(current, &edited, &field)?;
        *lookup_mut(document, &field).expect("selected leaf exists") = replacement;
    }
    Ok(())
}
