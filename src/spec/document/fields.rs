// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Fixed-shape TOML access shared by source mapping and its editing client.

use toml::{Table, Value};

pub(crate) fn lookup<'a>(table: &'a Table, field: &str) -> Option<&'a Value> {
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

pub(crate) fn lookup_mut<'a>(table: &'a mut Table, field: &str) -> Option<&'a mut Value> {
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

pub(crate) fn validate_shape(original: &Table, edited: &Table) -> Result<(), String> {
    check_table(original, edited, "")
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

pub(crate) fn path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_owned()
    } else {
        format!("{parent}.{key}")
    }
}
