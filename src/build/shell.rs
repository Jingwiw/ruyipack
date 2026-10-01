// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Locate a retained recipe result, then compose its recorded backend and engine.

use super::{Backend, Engine, compose::Compose, directory, invalid, mock::Mock, regular_file};
use clap::Args;
use fs_err as fs;
use serde::Deserialize;
use std::{io, path::PathBuf, time::Duration};

#[derive(Args)]
pub(crate) struct Options {
    /// Workspace development area; with --dir, an explicit build receipt's recipe key.
    #[arg(value_name = "WORK")]
    work: String,
    /// Advanced: look up DIR/RECIPE instead of the managed WORK/build result.
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
    /// Override Docker's current connection; the recorded daemon must still match.
    #[arg(long)]
    context: Option<String>,
    /// Deadline for each environment check and stop, not for the interactive shell.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
}

#[derive(Deserialize)]
struct Receipt {
    format_version: u32,
    package: String,
    backend: String,
    engine: String,
    context: Option<String>,
    config: PathBuf,
    execution: Execution,
}
#[derive(Deserialize)]
struct Execution {
    details: serde_json::Value,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    crate::check::metadata::Field::Name
        .validate(&options.work)
        .map_err(invalid)?;
    // The explicit --dir route is an advanced receipt consumer, never a missing-WORK fallback.
    let development = if options.dir.is_none() {
        let workspace = crate::workspace::discover()?;
        Some(workspace.existing_development(&options.work)?)
    } else {
        None
    };
    let package = development
        .as_ref()
        .map_or(options.work.as_str(), |area| area.package());
    let requested_output = match &development {
        Some(area) => area.directory().join("build"),
        None => options
            .dir
            .as_ref()
            .expect("explicit receipt directory")
            .join(&options.work),
    };
    let output = directory(&requested_output)?;
    let path = output.join("receipt.json");
    regular_file(&path)?;
    // Serialize shell sessions so a second opener cannot pass the stopped-worker
    // check just before the first starts it. Keep the lock until stop completes.
    let file = fs::File::open(&path)?.into_file();
    file.try_lock().map_err(|error| {
        io::Error::other(format!(
            "{}: another shell session owns this build: {error}",
            path.display()
        ))
    })?;
    let receipt: Receipt = serde_json::from_reader(&file).map_err(io::Error::other)?;
    if receipt.format_version != 1 || receipt.package != package {
        return Err(invalid("build receipt does not identify this recipe"));
    }
    let engine: &dyn Engine = match receipt.engine.as_str() {
        "mock" => &Mock,
        _ => return Err(invalid("unsupported recorded build engine")),
    };
    let context = options.context.as_deref().or(receipt.context.as_deref());
    if context.is_some_and(|c| c.trim().is_empty()) {
        return Err(invalid("Docker context must not be empty"));
    }
    let backend: &dyn Backend = match receipt.backend.as_str() {
        "compose" => &Compose {
            context,
            config: &receipt.config,
            remove: false,
        },
        _ => return Err(invalid("unsupported recorded build backend")),
    };
    // Never replay configuration paths or recovery commands from a receipt.
    backend.shell(
        &output,
        &engine.shell(),
        &receipt.execution.details,
        Duration::from_secs(options.timeout),
    )
}
