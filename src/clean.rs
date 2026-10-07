// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Remove retained resources, never a development recipe or authoring input.

use crate::build::CleanReport;
use serde::Serialize;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use clap::Args;

use crate::output_cli::{HumanLevel, ReportFormat};

#[derive(Args)]
pub(crate) struct Options {
    #[command(flatten)]
    input: crate::build::history::Selection,
    /// Remove the receipt-bound OBS package; retain WORK files and the OBS project.
    #[arg(long, requires = "work", conflicts_with_all = ["history", "build_dir", "attempt", "context", "timeout"])]
    remote: bool,
    /// Clean archived build attempts only; retain the current build and all authoring files.
    #[arg(long, requires = "work", conflicts_with_all = ["build_dir", "attempt"])]
    history: bool,
    /// Skip confirmation; resource ownership, daemon identity and locks still apply.
    #[arg(long)]
    force: bool,
    /// Override Docker's current connection; the recorded daemon must still match.
    #[arg(long)]
    context: Option<String>,
    /// Whole cleanup execution deadline.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    if options.remote {
        return clean_remote(
            options.input.work.as_deref().expect("remote requires WORK"),
            options,
        );
    }
    let selected = options.input.resolve()?;
    let development = selected.development;
    let path = selected.path;
    if options.history {
        let history = development
            .as_ref()
            .expect("history requires WORK")
            .directory()
            .join("build-history");
        let mut paths = Vec::new();
        match std::fs::symlink_metadata(&history) {
            Ok(metadata) if metadata.is_dir() => {
                for entry in std::fs::read_dir(&history)? {
                    paths.push(entry?.path());
                }
            }
            Ok(_) => {
                return Err(io::Error::other(
                    "build-history must be a directory, not a symlink",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        paths.sort();
        return run_history(&paths, options);
    }
    let report = run_build(&path, options, Duration::from_secs(options.timeout));
    match options.format {
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
        ReportFormat::Human => {
            if report.success {
                writeln!(io::stdout().lock(), "cleaned: {}", path.display())?;
            } else {
                print_removed(&report)?;
                writeln!(
                    io::stderr().lock(),
                    "clean failed: {}",
                    report.error.as_deref().unwrap_or("unknown error")
                )?;
            }
        }
    }
    Ok(report.success)
}

fn run_history(paths: &[PathBuf], options: &Options) -> io::Result<bool> {
    let start = Instant::now();
    let report = crate::batch::run(
        "clean-history",
        paths,
        |path| {
            let result = run_build(
                path,
                options,
                Duration::from_secs(options.timeout).saturating_sub(start.elapsed()),
            );
            if result.cancelled {
                std::ops::ControlFlow::Break(result)
            } else {
                std::ops::ControlFlow::Continue(result)
            }
        },
        |result| result.success,
    );
    match options.format {
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
        ReportFormat::Human => {
            for result in &report.results {
                if let Some(error) = &result.error {
                    print_removed(result)?;
                    writeln!(
                        io::stderr().lock(),
                        "clean failed: {}: {error}",
                        result.display_path
                    )?;
                }
            }
            writeln!(
                io::stdout().lock(),
                "cleaned history: {}/{}",
                report.results.iter().filter(|r| r.success).count(),
                paths.len()
            )?;
            if !report.pending.is_empty() {
                writeln!(
                    io::stderr().lock(),
                    "clean cancelled: {} attempt(s) not processed",
                    report.pending.len()
                )?;
            }
        }
    }
    Ok(report.success)
}

fn run_build(path: &Path, options: &Options, timeout: Duration) -> CleanReport {
    if let Err(error) =
        crate::output_cli::require_confirmation(options.format, options.force, "clean")
    {
        let mut report = crate::build::clean_report(path, options.context.as_deref());
        report.error = Some(error.to_string());
        return report;
    }
    crate::build::clean_result(
        path,
        options.context.as_deref(),
        timeout,
        &mut |report| {
            if matches!(options.format, ReportFormat::Human) {
                writeln!(
                    io::stderr().lock(),
                    "Clean results: {}",
                    crate::output_cli::human_path(path).display()
                )?;
                if let (Some(project), Some(daemon)) = (&report.project, &report.daemon_id) {
                    writeln!(
                        io::stderr().lock(),
                        "Docker project: {project}\nDocker daemon: {daemon}\nOwned resources: {:?}\nImages and volumes without Compose ownership labels are retained.",
                        report.scope
                    )?;
                }
            }
            crate::output_cli::confirm_removal(
                options.force,
                "clean",
                "Delete these resources and the result directory?",
            )
        },
        false,
    )
}

pub(crate) fn print_removed(report: &CleanReport) -> io::Result<()> {
    let mut out = crate::output_cli::stderr();
    for (kind, names) in &report.removed {
        for name in names {
            out.message(
                HumanLevel::Info,
                None,
                format_args!("removed {kind}: {name}"),
            )?;
        }
    }
    for image in &report.retained_images {
        out.message(
            HumanLevel::Info,
            None,
            format_args!("retained shared image: {image}"),
        )?;
    }
    Ok(())
}

#[derive(Serialize)]
struct RemoteReport<'a> {
    operation: &'static str,
    work: &'a str,
    success: bool,
    obs: crate::remote_build::cleanup::Report,
    error: Option<String>,
}

fn execute_remote<'a>(
    work: &'a str,
    authorize: &mut dyn FnMut() -> io::Result<()>,
) -> RemoteReport<'a> {
    let mut obs = crate::remote_build::cleanup::Report::default();
    let result = (|| {
        let workspace = crate::workspace::discover()?;
        let area = workspace.existing_development(work)?;
        authorize()?;
        area.verify_binding()?;
        crate::remote_build::cleanup::execute(&workspace, &area, false, &mut obs)
            .map_err(io::Error::other)
    })();
    RemoteReport {
        operation: "clean",
        work,
        success: result.is_ok(),
        obs,
        error: result.err().map(|e: io::Error| e.to_string()),
    }
}

fn clean_remote(work: &str, options: &Options) -> io::Result<bool> {
    let report = execute_remote(work, &mut || {
        crate::output_cli::require_confirmation(options.format, options.force, "clean")?;
        crate::output_cli::confirm_removal(
            options.force,
            "clean",
            "Remove this WORK's OBS package?",
        )
    });
    match options.format {
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
        ReportFormat::Human => {
            print_obs(work, &report.obs)?;
            if let Some(error) = &report.error {
                crate::output_cli::stderr().message(
                    HumanLevel::Error,
                    Some(Path::new(work)),
                    format_args!("{error}"),
                )?;
            } else if report.obs.target.is_none() {
                crate::output_cli::stderr().message(
                    HumanLevel::Info,
                    Some(Path::new(work)),
                    format_args!("Nothing to clean"),
                )?;
            }
        }
    }
    Ok(report.success)
}

pub(crate) fn print_obs(
    work: &str,
    report: &crate::remote_build::cleanup::Report,
) -> io::Result<()> {
    let mut out = crate::output_cli::stderr();
    if let Some(target) = &report.target {
        out.message(
            HumanLevel::Info,
            Some(Path::new(work)),
            format_args!("OBS: {target}; removed={}", report.package_removed),
        )?;
    }
    for reason in &report.retained {
        out.message(
            HumanLevel::Info,
            Some(Path::new(work)),
            format_args!("retained: {reason}"),
        )?;
    }
    Ok(())
}
