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
    after_help = "Shared OBS projects and images are retained. Use clean WORK to keep package files."
)]
pub(crate) struct Options {
    /// Existing development area to remove, including manifest, saved edits and downloaded sources.
    work: String,
    #[command(flatten)]
    arguments: Arguments,
}

#[derive(Args)]
pub(crate) struct Arguments {
    #[command(flatten)]
    request: Request,
    /// Skip confirmation; ownership checks still block deletion.
    #[arg(long)]
    force: bool,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

#[derive(Args)]
pub(crate) struct Request {
    /// Show the local deletion plan without contacting OBS or Docker.
    #[arg(long, conflicts_with = "force")]
    dry_run: bool,
    /// Override Docker's connection; every recorded daemon must still match.
    #[arg(long)]
    context: Option<String>,
    /// Build-resource cleanup deadline; OBS requests use their own network deadline.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
}

impl Arguments {
    pub(crate) fn execute(&self, work: &str) -> Report {
        execute(work, &self.request, &mut |report| {
            crate::output_cli::require_confirmation(self.format, self.force, "delete")?;
            if matches!(self.format, ReportFormat::Human) {
                report.print_scope()?;
            }
            crate::output_cli::confirm_removal(
                self.force,
                "delete",
                "Delete this development area?",
            )
        })
    }
}

#[derive(Serialize)]
pub(crate) struct Report {
    format_version: u32,
    operation: &'static str,
    work: String,
    preview: bool,
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
    options
        .arguments
        .execute(&options.work)
        .print(options.arguments.format)
}

pub(crate) fn execute(
    work: &str,
    options: &Request,
    authorize: &mut dyn FnMut(&Report) -> io::Result<()>,
) -> Report {
    let mut report = Report {
        format_version: 1,
        operation: "delete",
        work: work.to_owned(),
        preview: options.dry_run,
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
    match perform(options, &mut report, authorize) {
        Ok(()) => report.success = true,
        Err(error) => {
            report.cancelled = error.kind() == io::ErrorKind::Interrupted;
            report.error = Some(error.to_string());
        }
    }
    report
}

impl Report {
    fn print_scope(&self) -> io::Result<()> {
        use std::io::Write;
        let mut out = io::stderr().lock();
        writeln!(out, "Delete WORK {}", self.work)?;
        for path in &self.authoring_files {
            writeln!(out, "  {}", human_path(path).display())?;
        }
        Ok(())
    }

    pub(crate) fn print(&self, format: ReportFormat) -> io::Result<bool> {
        let report = self;
        match format {
            ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
            ReportFormat::Human => {
                if report.preview {
                    report.print_scope()?;
                }
                let mut out = crate::output_cli::stderr();
                crate::clean::print_obs(&report.work, &report.obs)?;
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

fn perform(
    options: &Request,
    report: &mut Report,
    authorize: &mut dyn FnMut(&Report) -> io::Result<()>,
) -> io::Result<()> {
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
    if options.dry_run {
        crate::remote_build::cleanup::execute(&workspace, &area, true, &mut report.obs)
            .map_err(io::Error::other)?;
        return Ok(());
    }
    authorize(report)?;
    // Recheck after confirmation and before each destructive phase. The WORK lock
    // protects cooperating commands; this is not a transaction against external writers.
    area.verify_binding()?;
    crate::remote_build::cleanup::execute(&workspace, &area, false, &mut report.obs)
        .map_err(io::Error::other)?;
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
