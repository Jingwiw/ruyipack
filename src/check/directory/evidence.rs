// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Match one WORK to a completed directory check; other packages may have failed.

use super::Check;
use serde::Deserialize;
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Receipt {
    format_version: u32,
    operation: String,
    directory: PathBuf,
    inputs: crate::workspace::baseline::Files,
    config: PathBuf,
    config_sha256: String,
    driver_sha256: String,
    execution: Execution,
    checks: Vec<Check>,
    result_error: Option<String>,
}

#[derive(Deserialize)]
struct Execution {
    success: bool,
}

fn matches(path: &Path, expected: &str) -> io::Result<()> {
    let actual = crate::file_digest::read(path).map_err(io::Error::other)?;
    if actual.sha256 != expected {
        return Err(io::Error::other(format!(
            "{}: check evidence is stale",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn verify(path: &Path, area: &crate::workspace::Development) -> io::Result<()> {
    let receipt: Receipt = crate::workspace::baseline::load(path)?;
    if receipt.format_version != 1
        || receipt.operation != "directory-check"
        || !receipt.execution.success
        || receipt.result_error.is_some()
        || !receipt.inputs.contains_key("scripts/remoteassetify.py")
        || !receipt.inputs.contains_key(".pre-commit-config.yaml")
        || receipt.driver_sha256 != crate::utf8_file::sha256(include_str!("../runner.py"))
        || !super::complete(&receipt.inputs, &receipt.checks)
    {
        return Err(io::Error::other(
            "Directory check is incomplete or incompatible.",
        ));
    }
    if receipt.checks[0].exit_code != 0 {
        return Err(io::Error::other(
            "Prepared-directory pre-commit checks failed.",
        ));
    }
    matches(
        &receipt.directory.join(".pre-commit-config.yaml"),
        &receipt.inputs[".pre-commit-config.yaml"].sha256,
    )?;
    match_tree(
        &receipt.directory.join("scripts"),
        "scripts/",
        &receipt.inputs,
    )?;
    matches(&receipt.config, &receipt.config_sha256)?;
    let spec = area.spec()?;
    let name = spec
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("SPEC filename must be UTF-8"))?;
    let selected = format!("SPECS/{}/{name}", area.package());
    if !receipt
        .checks
        .iter()
        .any(|check| check.path == selected && check.exit_code == 0)
    {
        return Err(io::Error::other(format!(
            "{selected}: no successful directory check"
        )));
    }
    let prefix = format!("SPECS/{}/", area.package());
    match_tree(area.package_directory(), &prefix, &receipt.inputs)?;
    match_tree(
        &receipt.directory.join("SPECS").join(area.package()),
        &prefix,
        &receipt.inputs,
    )
}

fn match_tree(
    root: &Path,
    prefix: &str,
    inputs: &crate::workspace::baseline::Files,
) -> io::Result<()> {
    let expected: crate::workspace::baseline::Files = inputs
        .iter()
        .filter_map(|(name, file)| {
            name.strip_prefix(prefix)
                .map(|name| (name.to_owned(), file.clone()))
        })
        .collect();
    if crate::workspace::baseline::read(root)? != expected {
        return Err(io::Error::other("Check input evidence is stale."));
    }
    Ok(())
}
