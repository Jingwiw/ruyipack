// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Field selection and shape checks for an existing editable document.

use std::{borrow::Cow, collections::BTreeSet};

use serde_json::{Value as Json, json};
use toml::{Table, Value};

use crate::spec::document::fields::{lookup, lookup_mut, path};

/// Replaces existing string fields without inferring types from their spelling.
pub(super) fn assign<'a>(
    original: &'a Table,
    assignments: &[(String, String)],
) -> Result<Cow<'a, Table>, String> {
    let mut document = Cow::Borrowed(original);
    let mut seen = BTreeSet::new();
    for (field, value) in assignments {
        if !seen.insert(field) {
            return Err(format!("{field}: repeated assignment"));
        }
        let target = lookup_mut(document.to_mut(), field)
            .ok_or_else(|| format!("{field}: unknown field"))?;
        if !target.is_str() {
            return Err(format!(
                "{field}: direct assignment requires a string field; use the editor for arrays or groups"
            ));
        }
        *target = Value::String(value.clone());
    }
    Ok(document)
}

/// Validates field names without constructing an editable projection.
pub(super) fn validate_selection(original: &Table, selected: &[String]) -> Result<(), String> {
    for field in selected {
        if lookup(original, field).is_none() {
            return Err(format!("{field}: unknown field or group"));
        }
    }
    Ok(())
}

/// Describes the current draft, not fields unsupported by the source mapping.
pub(super) fn schema(document: &Table) -> Json {
    let mut schema = table_schema(document, "");
    schema["$schema"] = "http://json-schema.org/draft-07/schema#".into();
    schema["title"] = "SPEC edit draft".into();
    schema["description"] = "Editable fields from the current SPEC. Source expressions are not macro-expanded; SPEC validation runs after editing.".into();
    schema
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
