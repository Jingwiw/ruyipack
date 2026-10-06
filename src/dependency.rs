// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Reversible shorthand for simple RPM capabilities. Complex expressions stay literal.

pub(crate) const NAMESPACES: &[&str] = &[
    "rpm",
    "pkgconfig",
    "cmake",
    "python3dist",
    "perl",
    "rubygem",
];

pub(crate) fn split(value: &str) -> (&str, String) {
    for &namespace in &NAMESPACES[1..] {
        if let Some(rest) = value
            .strip_prefix(namespace)
            .and_then(|s| s.strip_prefix('('))
            && let Some((name, suffix)) = rest.split_once(')')
            && !name.is_empty()
            && !name.contains(['(', '%', ' ', '\t'])
            && (suffix.is_empty()
                || suffix.starts_with(" >= ")
                || suffix.starts_with(" <= ")
                || suffix.starts_with(" = ")
                || suffix.starts_with(" > ")
                || suffix.starts_with(" < "))
            && !suffix
                .trim()
                .split_once(' ')
                .is_some_and(|(_, version)| version.chars().any(char::is_whitespace))
        {
            return (namespace, format!("{name}{suffix}"));
        }
    }
    ("rpm", value.to_owned())
}

pub(crate) fn expand(namespace: &str, value: &str) -> Result<String, String> {
    if namespace == "rpm" {
        return Ok(value.to_owned());
    }
    if !NAMESPACES.contains(&namespace) {
        return Err(format!("unknown dependency namespace: {namespace}"));
    }
    let end = value.find(char::is_whitespace).unwrap_or(value.len());
    let result = format!("{namespace}({}){}", &value[..end], &value[end..]);
    if split(&result) != (namespace, value.to_owned()) {
        return Err(format!(
            "{namespace}: expected a capability name and optional version constraint; keep complex expressions in rpm"
        ));
    }
    Ok(result)
}
