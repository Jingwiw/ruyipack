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
    #[command(flatten)]
    input: super::history::Selection,
    /// Run a command in the Mock build directory instead of opening an interactive shell.
    #[arg(last = true, conflicts_with = "export_patch")]
    command: Vec<String>,
    /// Export selected text-file changes from a successful prep baseline; does not modify SPEC.
    #[arg(long, requires_all = ["source_root", "paths"])]
    export_patch: Option<PathBuf>,
    /// Source subtree relative to the native RPM build directory (for example busybox-1.37.0).
    #[arg(long, requires = "export_patch", value_hint = clap::ValueHint::Other)]
    source_root: Option<PathBuf>,
    /// File relative to --source-root; repeat to select changes, including additions/deletions.
    #[arg(long = "path", requires = "export_patch", value_hint = clap::ValueHint::Other)]
    paths: Vec<PathBuf>,
    /// Override Docker's current connection; the recorded daemon must still match.
    #[arg(long)]
    context: Option<String>,
    /// Deadline for each check, command or stop; interactive shells remain unbounded.
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
    resources_retained: Option<bool>,
}
#[derive(Deserialize)]
struct Execution {
    details: serde_json::Value,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    let selected = options.input.resolve()?;
    let development = selected.development;
    let output = directory(&selected.path)?;
    let path = output.join("receipt.json");
    regular_file(&path)?;
    // Serialize shell sessions so a second opener cannot pass the stopped-worker
    // check just before the first starts it. Keep the lock until stop completes.
    let file = fs::File::open(&path)?.into_file();
    let lock = crate::file_lock::FileLock::try_lock(file).map_err(|error| {
        io::Error::other(format!(
            "{}: another shell session owns this build: {error}",
            path.display()
        ))
    })?;
    let receipt: Receipt = serde_json::from_reader(lock.file()).map_err(io::Error::other)?;
    if receipt.resources_retained == Some(false) {
        return Err(invalid(
            "this historical attempt no longer owns the worker; use the current build",
        ));
    }
    if receipt.format_version != 1
        || development
            .as_ref()
            .is_some_and(|area| receipt.package != area.package())
    {
        return Err(invalid("build receipt does not identify this recipe"));
    }
    let engine: &dyn Engine = match receipt.engine.as_str() {
        "mock" => &Mock(super::Stage::Build),
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
            probe: None,
        },
        _ => return Err(invalid("unsupported recorded build backend")),
    };
    if options.command.is_empty() && options.export_patch.is_none() {
        crate::output_cli::stderr().message(
            crate::output_cli::HumanLevel::Info,
            None,
            format_args!("Mock chroot; changes remain here until explicitly exported"),
        )?;
    }
    if let Some(area) = &development {
        crate::output_cli::stderr().message(
            crate::output_cli::HumanLevel::Debug,
            None,
            format_args!("recipe directory {}", area.package_directory().display()),
        )?;
    }
    // Never replay configuration paths or recovery commands from a receipt.
    let mut invocation = if options.export_patch.is_some() {
        engine.export_patch()
    } else {
        engine.shell()
    };
    if options.export_patch.is_some() {
        invocation.extend([
            "--source-root".into(),
            options
                .source_root
                .as_ref()
                .expect("clap requires source root")
                .to_string_lossy()
                .into_owned(),
        ]);
        for path in &options.paths {
            invocation.extend(["--path".into(), path.to_string_lossy().into_owned()]);
        }
    }
    if !options.command.is_empty() {
        invocation.push("--".into());
        invocation.extend(options.command.iter().cloned());
    }
    backend.shell(
        &output,
        &invocation,
        &receipt.execution.details,
        options.export_patch.as_deref(),
        Duration::from_secs(options.timeout),
    )
}
