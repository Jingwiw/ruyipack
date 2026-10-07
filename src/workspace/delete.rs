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
    after_help = "Removes WORK files and its owned OBS package and build resources. Use --only obs or --only build to keep WORK.\nPreview with --dry-run. Ownership conflicts block deletion, even with --force. Recipe files are listed for explicit deletion. Git repositories and commits are not removed.\nTo keep package changes and only remove build results, use clean WORK."
)]
pub(crate) struct Options {
    /// Existing development area to remove, including manifest, saved edits and downloaded sources.
    work: String,
    #[command(flatten)]
    arguments: Arguments,
}

#[derive(Args)]
pub(crate) struct Arguments {
    /// Remove only these external resources; keep the WORK and its recipe.
    #[arg(long, value_enum)]
    only: Option<Scope>,
    /// Show the local deletion plan without contacting OBS or Docker.
    #[arg(long, conflicts_with = "force")]
    dry_run: bool,
    /// Skip confirmation; ownership checks still block deletion.
    #[arg(long)]
    force: bool,
    /// Override Docker's connection; every recorded daemon must still match.
    #[arg(long)]
    context: Option<String>,
    /// Build-resource cleanup deadline; OBS requests use their own network deadline.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Scope {
    Obs,
    Build,
}

#[derive(Serialize)]
pub(crate) struct Report {
    format_version: u32,
    operation: &'static str,
    work: String,
    preview: bool,
    only: Option<Scope>,
    obs: crate::remote_build::cleanup::Report,
    pub(crate) success: bool,
    directory: Option<PathBuf>,
    authoring_files: Vec<PathBuf>,
    builds: Vec<PathBuf>,
    completed: Vec<PathBuf>,
    cleanup: Vec<crate::build::CleanReport>,
    pub(crate) cancelled: bool,
    error: Option<String>,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    execute(&options.work, &options.arguments).print(options.arguments.format)
}

pub(crate) fn execute(work: &str, options: &Arguments) -> Report {
    let mut report = Report {
        format_version: 1,
        operation: "delete",
        work: work.to_owned(),
        preview: options.dry_run,
        only: options.only,
        obs: crate::remote_build::cleanup::Report::default(),
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
    report
}

impl Report {
    pub(crate) fn print(&self, format: ReportFormat) -> io::Result<bool> {
        let report = self;
        match format {
            ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
            ReportFormat::Human => {
                let mut out = crate::output_cli::stderr();
                if let Some(target) = &report.obs.target {
                    out.message(
                        HumanLevel::Info,
                        Some(Path::new(&report.work)),
                        format_args!("OBS: {target}; removed={}", report.obs.package_removed),
                    )?;
                }
                for reason in &report.obs.retained {
                    out.message(
                        HumanLevel::Info,
                        Some(Path::new(&report.work)),
                        format_args!("retained: {reason}"),
                    )?;
                }
                for cleanup in &report.cleanup {
                    crate::clean::print_removed(cleanup)?;
                }
                if let Some(error) = &report.error {
                    out.message(
                        HumanLevel::Error,
                        Some(Path::new(&report.work)),
                        format_args!("{error}"),
                    )?;
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
                        Some(Path::new(&report.work)),
                        format_args!(
                            "{}",
                            if report.preview {
                                "deletion preview; nothing removed"
                            } else if report.only.is_some() {
                                "cleanup finished; WORK retained"
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
}

fn perform(options: &Arguments, report: &mut Report) -> io::Result<()> {
    let workspace = super::discover()?;
    let area = workspace.existing_development(&report.work)?;
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
            Some("build") if options.only != Some(Scope::Obs) => report.builds.push(path),
            Some("build-history") if options.only != Some(Scope::Obs) => {
                if !entry.file_type()?.is_dir() {
                    return Err(invalid(
                        "build-history must not be a symlink or non-directory",
                    ));
                }
                for entry in fs::read_dir(path)? {
                    report.builds.push(entry?.path());
                }
            }
            _ if options.only.is_none() => inventory(&path, &mut report.authoring_files)?,
            _ => (),
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
            report.work,
            human_path(root).display()
        );
        for path in &report.authoring_files {
            eprintln!("  {}", human_path(path).display());
        }
        eprintln!(
            "Selected resources will be removed. Shared resources and the recipe repository are retained."
        );
    }
    if options.dry_run {
        if options.only != Some(Scope::Build) {
            crate::remote_build::cleanup::execute(&workspace, &area, true, &mut report.obs)
                .map_err(io::Error::other)?;
        }
        return Ok(());
    }
    crate::output_cli::confirm_removal(options.force, "delete", "Delete the selected resources?")?;
    // Recheck after confirmation and before each destructive phase. The WORK lock
    // protects cooperating commands; this is not a transaction against external writers.
    area.verify_binding()?;
    if options.only != Some(Scope::Build) {
        crate::remote_build::cleanup::execute(&workspace, &area, false, &mut report.obs)
            .map_err(io::Error::other)?;
    }
    let start = Instant::now();
    let timeout = Duration::from_secs(options.timeout);
    for path in &report.builds {
        let result = crate::build::clean_result(
            path,
            options.context.as_deref(),
            timeout.saturating_sub(start.elapsed()),
            &mut |_| Ok(()),
            true,
        );
        let error = result.error.clone();
        report.cleanup.push(result);
        if let Some(error) = error {
            return Err(io::Error::other(error));
        }
        report.completed.push(path.clone());
    }
    area.verify_binding()?;
    if options.only.is_none() {
        fs::remove_dir_all(root)?;
        report.completed.push(root.to_owned());
    }
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
