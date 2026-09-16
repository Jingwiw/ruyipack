// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Field selection and shape checks for an existing editable document.

use std::collections::BTreeSet;

use serde_json::{Value as Json, json};
use toml::{Table, Value};

/// Replaces existing string fields without inferring types from their spelling.
pub(super) fn assign(original: &Table, assignments: &[(String, String)]) -> Result<Table, String> {
    let mut document = original.clone();
    let mut seen = BTreeSet::new();
    for (field, value) in assignments {
        if !seen.insert(field) {
            return Err(format!("{field}: repeated assignment"));
        }
        let target =
            lookup_mut(&mut document, field).ok_or_else(|| format!("{field}: unknown field"))?;
        if !target.is_str() {
            return Err(format!(
                "{field}: direct assignment requires a string field; use the editor for arrays or groups"
            ));
        }
        *target = Value::String(value.clone());
    }
    Ok(document)
}

/// Selects fields or complete groups while retaining the original table order.
pub(super) fn select(original: &Table, selected: &[String]) -> Result<Table, String> {
    for field in selected {
        if lookup(original, field).is_none() {
            return Err(format!("{field}: unknown field or group"));
        }
    }
    if selected.is_empty() {
        return Ok(original.clone());
    }
    Ok(select_table(original, selected, ""))
}

/// Merges only the selected shape; unselected or unknown input is an error.
pub(super) fn merge(
    original: &Table,
    selected: &[String],
    edited: &Table,
) -> Result<Table, String> {
    let projection = select(original, selected)?;
    validate_shape(&projection, edited)?;
    let mut result = original.clone();
    merge_table(&mut result, edited);
    Ok(result)
}

/// Describes the current draft, not fields unsupported by the source mapping.
pub(super) fn schema(document: &Table) -> Json {
    let mut schema = table_schema(document, "");
    schema["$schema"] = "http://json-schema.org/draft-07/schema#".into();
    schema["title"] = "SPEC edit draft".into();
    schema["description"] = "Editable fields from the current SPEC. Source expressions are not macro-expanded; SPEC validation runs after editing.".into();
    schema
}

pub(super) fn lookup<'a>(table: &'a Table, field: &str) -> Option<&'a Value> {
    let (head, tail) = field
        .split_once('.')
        .map_or((field, None), |(head, tail)| (head, Some(tail)));
    if head.is_empty() {
        return None;
    }
    let value = table.get(head)?;
    match tail {
        Some(tail) => lookup(value.as_table()?, tail),
        None => Some(value),
    }
}

pub(super) fn lookup_mut<'a>(table: &'a mut Table, field: &str) -> Option<&'a mut Value> {
    let (head, tail) = field
        .split_once('.')
        .map_or((field, None), |(head, tail)| (head, Some(tail)));
    if head.is_empty() {
        return None;
    }
    let value = table.get_mut(head)?;
    match tail {
        Some(tail) => lookup_mut(value.as_table_mut()?, tail),
        None => Some(value),
    }
}

pub(super) fn validate_shape(original: &Table, edited: &Table) -> Result<(), String> {
    check_table(original, edited, "")
}

fn select_table(original: &Table, selected: &[String], parent: &str) -> Table {
    let mut result = Table::new();
    for (key, value) in original {
        let field = path(parent, key);
        if selected.iter().any(|selected| selected == &field) {
            result.insert(key.clone(), value.clone());
        } else if selected
            .iter()
            .any(|selected| selected.starts_with(&format!("{field}.")))
            && let Value::Table(table) = value
        {
            result.insert(
                key.clone(),
                Value::Table(select_table(table, selected, &field)),
            );
        }
    }
    result
}

fn merge_table(original: &mut Table, edited: &Table) {
    for (key, changed) in edited {
        let value = original.get_mut(key).expect("validated field shape");
        match (value, changed) {
            (Value::Table(original), Value::Table(edited)) => merge_table(original, edited),
            (value, changed) => *value = changed.clone(),
        }
    }
}

fn check_table(original: &Table, edited: &Table, parent: &str) -> Result<(), String> {
    for (key, value) in original {
        let field = path(parent, key);
        let changed = edited.get(key).ok_or_else(|| {
            format!("{field}: deleting an existing field or table is unsupported")
        })?;
        match (value, changed) {
            (Value::Table(original), Value::Table(edited)) => {
                check_table(original, edited, &field)?
            }
            (Value::Array(_), Value::Array(edited)) if edited.iter().all(Value::is_str) => {}
            (Value::String(_), Value::String(_)) => {}
            (Value::Array(_), _) => return Err(format!("{field}: expected a string array")),
            (Value::Table(_), _) => return Err(format!("{field}: expected a table")),
            (Value::String(_), _) => return Err(format!("{field}: expected a string")),
            _ => return Err(format!("{field}: unsupported draft value type")),
        }
    }
    for key in edited.keys() {
        if !original.contains_key(key) {
            return Err(format!(
                "{}: new, unknown or unselected field",
                path(parent, key)
            ));
        }
    }
    Ok(())
}

fn table_schema(table: &Table, parent: &str) -> Json {
    let mut properties = serde_json::Map::new();
    for (key, value) in table {
        let field = path(parent, key);
        let mut schema = match value {
            Value::Table(table) => table_schema(table, &field),
            Value::Array(_) => json!({ "type": "array", "items": { "type": "string" } }),
            Value::String(_) => json!({ "type": "string" }),
            // The source mapper currently emits only strings, string arrays and tables.
            _ => Json::Bool(false),
        };
        if schema.is_object() {
            schema["description"] = description(&field).into();
        }
        properties.insert(key.clone(), schema);
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": table.keys().collect::<Vec<_>>(),
        "additionalProperties": false,
    })
}

fn path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_owned()
    } else {
        format!("{parent}.{key}")
    }
}

fn description(field: &str) -> &'static str {
    match field {
        "package" => "Main-package metadata and file lists.",
        "package.name" => "RPM Name source expression.",
        "package.version" => {
            "RPM Version source expression; changing it does not verify source archives or patches."
        }
        "package.summary" => "Main-package Summary source text.",
        "package.license" => {
            "License expression for the packaged software, separate from the SPEC file license."
        }
        "package.url" => "Project homepage from the URL tag.",
        "package.description" => "Main-package description body, including its line endings.",
        "package.files" => "Main-package file lists; entries retain their RPM source expressions.",
        "package.files.license" => "Paths marked with %license.",
        "package.files.doc" => "Paths marked with %doc.",
        "package.files.entries" => "File paths without an additional file directive.",
        "spec" => "SPEC file metadata and text separate from the software metadata.",
        "spec.release" => "RPM Release source expression.",
        "spec.changelog" => "Changelog body, including its line endings.",
        "spec.license" => "License of the SPEC file itself.",
        "spec.copyright-years" => {
            "Shared copyright year or year range from the existing copyright declarations."
        }
        "spec.copyright-holders" => "Copyright holders from the SPEC header.",
        "spec.contributors" => "SPEC contributors from the existing header.",
        "spec.comments" => {
            "Existing ordinary comment blocks, including their # prefixes. Machine-readable declarations have separate fields."
        }
        "build" => "Existing declarative build settings.",
        "build.system" => "BuildSystem tag source expression.",
        "build-requires" => "Build dependencies declared in this SPEC.",
        "build-requires.rpm" => {
            "Existing BuildRequires source expressions; changing the list does not install or resolve dependencies."
        }
        "sources" => "Sources keyed by their RPM source number.",
        _ if field.starts_with("sources.") && field.ends_with(".url") => {
            "Source URL expression; RPM macros remain unexpanded."
        }
        _ if field.starts_with("sources.") && field.ends_with(".sha256") => {
            "SHA-256 declared by the adjacent RemoteAsset comment; editing does not download or verify the archive."
        }
        _ => "Field from the existing SPEC.",
    }
}
