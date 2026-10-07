// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Match one WORK to a completed directory check; other packages may have failed.

use super::{Check, File};
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
    script: File,
    inputs: Vec<File>,
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
        || receipt.script.path != "scripts/remoteassetify.py"
        || receipt.driver_sha256 != crate::utf8_file::sha256(include_str!("../runner.py"))
        || receipt.inputs.len() != receipt.checks.len()
        || receipt
            .inputs
            .iter()
            .zip(&receipt.checks)
            .any(|(input, check)| input.path != check.path)
    {
        return Err(io::Error::other(
            "Directory check is incomplete or incompatible.",
        ));
    }
    matches(
        &receipt.directory.join(&receipt.script.path),
        &receipt.script.sha256,
    )?;
    matches(&receipt.config, &receipt.config_sha256)?;
    let spec = area.spec()?;
    let name = spec
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("SPEC filename must be UTF-8"))?;
    let selected = format!("SPECS/{}/{name}", area.package());
    let mut entries = receipt
        .inputs
        .iter()
        .zip(&receipt.checks)
        .filter(|(input, _)| input.path == selected);
    let (input, check) = entries
        .next()
        .ok_or_else(|| io::Error::other(format!("{selected}: no directory check result")))?;
    if entries.next().is_some() || check.exit_code != 0 {
        return Err(io::Error::other(format!(
            "{selected}: directory check failed or is duplicated"
        )));
    }
    matches(&spec, &input.sha256)?;
    matches(&receipt.directory.join(&selected), &input.sha256)
}
