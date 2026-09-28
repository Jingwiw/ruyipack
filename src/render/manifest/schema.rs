// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! JSON Schema for authoring input, not the selected-field edit projection.

use super::ManifestInput;
use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings};
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) fn generate() -> Schema {
    SchemaSettings::draft07()
        .for_deserialize()
        .with_transform(schemars::transform::RecursiveTransform(toml_types))
        .into_generator()
        .into_root_schema_for::<ManifestInput>()
}

/// TOML has no null value; optional fields are omitted instead.
fn toml_types(schema: &mut Schema) {
    if let Some(serde_json::Value::Array(types)) = schema.get_mut("type") {
        types.retain(|value| value != "null");
    }
    if schema.get("default") == Some(&serde_json::Value::Null) {
        schema.remove("default");
    }
}

/// The wire form is a table, not the Vec used to preserve Patch order.
/// u32 parsing also accepts +1 and 01. Bounds and numeric alias collisions
/// remain gen checks; requiring literal key "0" would reject legal "00".
pub(super) fn materials<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = <BTreeMap<String, T>>::json_schema(generator);
    schema.insert("propertyNames".into(), json!({"pattern": r"^\+?[0-9]+$"}));
    schema.insert("description".into(), json!("Tables keyed by Source/Patch number. gen checks u32 bounds and duplicate numeric identities; sources must include number 0. Patch table declaration order is preserved."));
    schema
}
