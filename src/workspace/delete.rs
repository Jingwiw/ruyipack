// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Explicit disposal of one WORK; the build backend keeps ownership of its resources.

use super::invalid;
use crate::output_cli::{HumanLevel, ReportFormat, human_path};
use clap::Args;
use fs_err as fs;
use serde::Serialize;
use std::{
    io::{self},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Args)]
#[command(
    after_help = "Removes recipe files, authoring TOML, saved edits, downloads and owned build results.\nPreview with --dry-run. Ownership conflicts block deletion, even with --force. Recipe files are listed for explicit deletion. Git repositories and commits are not removed.\nTo keep package changes and only remove build results, use clean WORK."
)]
pub(crate) struct Options {
    /// Existing development area to remove, including manifest, saved edits and downloaded sources.
    work: String,
    /// Show the local deletion plan without contacting Docker or removing anything.
    #[arg(long, conflicts_with = "force")]
    dry_run: bool,
    /// Skip confirmation; ownership checks still block deletion.
    #[arg(long)]
    force: bool,
    /// Override Docker's connection; every recorded daemon must still match.
    #[arg(long)]
    context: Option<String>,
    /// Total build-resource cleanup deadline; Git operations retain their own bounded execution.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Serialize)]
struct Report {
    format_version: u32,
    operation: &'static str,
    work: String,
    preview: bool,
    success: bool,
    directory: Option<PathBuf>,
    authoring_files: Vec<PathBuf>,
    builds: Vec<PathBuf>,
    completed: Vec<PathBuf>,
    cleanup: Vec<crate::build::CleanReport>,
    cancelled: bool,
    error: Option<String>,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    let mut report = Report {
        format_version: 1,
        operation: "delete",
        work: options.work.clone(),
        preview: options.dry_run,
        success: false,
        directory: None,
        authoring_files: Vec::new(),
        builds: Vec::new(),
        completed: Vec::new(),
        cleanup: Vec::new(),
        cancelled: false,
        error: None,
    };
    match perform(options, &mut report) {
        Ok(()) => report.success = true,
        Err(error) => {
            report.cancelled = error.kind() == io::ErrorKind::Interrupted;
            report.error = Some(error.to_string());
        }
    }
    match options.format {
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
        ReportFormat::Human => {
            let mut out = crate::output_cli::stderr();
            if let Some(error) = &report.error {
                out.message(
                    HumanLevel::Error,
                    Some(Path::new(&options.work)),
                    format_args!("{error}"),
                )?;
                for cleanup in &report.cleanup {
                    crate::clean::print_removed(cleanup)?;
                }
                for path in &report.completed {
                    out.message(
                        HumanLevel::Info,
                        None,
                        format_args!("removed: {}", human_path(path).display()),
                    )?;
                }
            } else {
                out.message(
                    HumanLevel::Info,
                    Some(Path::new(&options.work)),
                    format_args!(
                        "{}",
                        if options.dry_run {
                            "deletion preview; nothing removed"
                        } else {
                            "development area deleted"
                        }
                    ),
                )?;
            }
        }
    }
    Ok(report.success)
}

fn perform(options: &Options, report: &mut Report) -> io::Result<()> {
    let workspace = super::discover()?;
    let area = workspace.existing_development(&options.work)?;
    let root = area.directory();
    if fs::canonicalize(std::env::current_dir()?)?.starts_with(root)
        || std::env::var_os("HOME").is_some_and(|home| Path::new(&home).starts_with(root))
    {
        return Err(invalid(
            "leave the development area before deleting it; HOME must not be inside it",
        ));
    }
    report.directory = Some(root.to_owned());
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        match entry.file_name().to_str() {
            Some("build") => report.builds.push(path),
            Some("build-history") => {
                if !entry.file_type()?.is_dir() {
                    return Err(invalid(
                        "build-history must not be a symlink or non-directory",
                    ));
                }
                for entry in fs::read_dir(path)? {
                    report.builds.push(entry?.path());
                }
            }
            _ => inventory(&path, &mut report.authoring_files)?,
        }
    }
    report.authoring_files.sort();
    report.builds.sort();
    for path in &report.builds {
        crate::build::preflight_cleanup(path)?;
    }
    if !options.dry_run {
        crate::output_cli::require_confirmation(options.format, options.force, "delete")?;
    }
    if matches!(options.format, ReportFormat::Human) {
        eprintln!(
            "Delete WORK {}: {}",
            options.work,
            human_path(root).display()
        );
        for path in &report.authoring_files {
            eprintln!("  {}", human_path(path).display());
        }
        eprintln!(
            "Recipe and the listed TOML and source files will be removed. Shared recipe repository and images are retained."
        );
    }
    if options.dry_run {
        return Ok(());
    }
    crate::output_cli::confirm_removal(options.force, "delete", "Delete this development area?")?;
    // Recheck after confirmation and before each destructive phase. The WORK lock
    // protects cooperating commands; this is not a transaction against external writers.
    area.verify_binding()?;
    let start = Instant::now();
    let timeout = Duration::from_secs(options.timeout);
    for path in &report.builds {
        let result = crate::build::clean_result(
            path,
            options.context.as_deref(),
            timeout.saturating_sub(start.elapsed()),
            &mut |_| Ok(()),
        );
        let error = result.error.clone();
        report.cleanup.push(result);
        if let Some(error) = error {
            return Err(io::Error::other(error));
        }
        report.completed.push(path.clone());
    }
    area.verify_binding()?;
    fs::remove_dir_all(root)?;
    report.completed.push(root.to_owned());
    Ok(())
}

fn inventory(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            inventory(&entry?.path(), files)?;
        }
    } else if metadata.is_file() {
        files.push(path.to_owned());
    } else {
        return Err(invalid(format!(
            "{} is a symlink or special file; remove it explicitly before deleting WORK",
            path.display()
        )));
    }
    Ok(())
}
