// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Advance package work; validation owns facts, tasks own pending actions.
mod execution;

use crate::{
    file_lock::FileLock,
    output_cli::{HumanLevel, ReportFormat},
    plan,
    remote_build::status::{self, State},
    workspace::{self, baseline, commit::validation},
};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub(crate) struct Options {
    /// Package plan. Repeat for OBS submission, status, commit or delete.
    #[arg(long, required = true, value_name = "PATH")]
    plan: Vec<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate changed packages and resume pending builds.
    Run(RunOptions),
    /// Submit selected packages and reconcile their shared OBS projects.
    RemoteBuild(crate::remote_build::SubmitOptions),
    /// Observe retained OBS submissions without uploading.
    Status {
        #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
        format: ReportFormat,
    },
    /// Commit each selected WORK separately; retain partial results.
    Commit(workspace::commit::Arguments),
    /// Delete selected WORKs and their owned resources; retain partial results.
    Delete(workspace::delete::Arguments),
    /// Preview, publish or close a PR for the selected committed packages.
    Pr(crate::workspace::pr::Options),
}

#[derive(Args)]
struct RunOptions {
    /// Require a directory-check receipt for each WORK. Retained for later runs.
    #[arg(long, value_name = "FILE")]
    check_receipt: Option<PathBuf>,
    /// Validation target. Defaults to the saved target, or local for a new task.
    #[arg(long, value_enum)]
    validation: Option<Validation>,
    /// Resume stopped validation after inspecting and correcting the WORK.
    #[arg(long)]
    retry: bool,
    /// Observe remote results before the next scheduled poll.
    #[arg(long)]
    poll_now: bool,
    /// Execution budget for each operation in seconds.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Validation {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Materials,
    Check,
    Build,
    Submit,
    Waiting,
    Validated,
    Unchanged,
    Paused,
    Failed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Task {
    work: String,
    check_receipt: Option<PathBuf>,
    remote: bool,
    phase: Phase,
    running: bool,
    attempt: u64,
    next_poll: i64,
    polls: u32,
    query_errors: u32,
    candidate: Option<baseline::Files>,
    validation_key: String,
    error: Option<String>,
    failed_step: Option<Phase>,
}
impl Task {
    fn new(work: String, options: &RunOptions) -> Self {
        Self {
            work,
            check_receipt: None,
            remote: options.validation == Some(Validation::Remote),
            phase: Phase::Materials,
            running: false,
            attempt: 0,
            next_poll: 0,
            polls: 0,
            query_errors: 0,
            candidate: None,
            validation_key: String::new(),
            error: None,
            failed_step: None,
        }
    }
    fn stop(&mut self, error: &impl ToString) {
        self.failed_step = Some(self.phase);
        self.phase = Phase::Failed;
        self.running = false;
        self.error = Some(error.to_string());
    }
}

#[derive(Serialize)]
struct Report {
    operation: &'static str,
    completed: bool,
    observed_at: i64,
    validated_plan: PathBuf,
    tasks: Vec<Task>,
}

fn claim(root: &Path, work: &str) -> io::Result<(PathBuf, FileLock)> {
    let path = workspace::directory(root, Path::new(work), false)?;
    std::fs::create_dir_all(&path)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.join("lock"))?;
    Ok((path, FileLock::try_lock(file).map_err(io::Error::other)?))
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    let plans = plan::load(&options.plan)?;
    if plans.iter().all(|plan| plan.packages.is_empty()) {
        #[derive(Serialize)]
        struct EmptySelection {
            operation: &'static str,
            success: bool,
            selected: usize,
        }
        let format = match &options.command {
            Command::Run(args) => args.format,
            Command::RemoteBuild(args) => args.format,
            Command::Status { format } => *format,
            Command::Commit(args) => args.format,
            Command::Delete(args) => args.format,
            Command::Pr(args) => args.format,
        };
        match format {
            ReportFormat::Toml => crate::report::write(
                &mut io::stdout().lock(),
                &EmptySelection {
                    operation: "task",
                    success: true,
                    selected: 0,
                },
            )?,
            ReportFormat::Human => crate::output_cli::stderr().message(
                HumanLevel::Info,
                None,
                format_args!("No WORK selected."),
            )?,
        }
        return Ok(true);
    }
    match &options.command {
        Command::RemoteBuild(args) => {
            let tasks = plans
                .into_iter()
                .flat_map(|p| {
                    p.packages
                        .into_iter()
                        .map(move |t| (t.work, Some(t.settings.inherit(&p.defaults))))
                })
                .collect();
            let report = crate::remote_build::submit(&workspace::discover()?, tasks, args)
                .map_err(io::Error::other)?;
            report.print(args.format).map_err(io::Error::other)
        }
        Command::Status { format } => {
            let works = selected_works(plans);
            status::run(&workspace::discover()?, &works, *format).map_err(io::Error::other)
        }
        Command::Commit(args) => {
            let works = selected_works(plans);
            let report = crate::batch::run(
                "commit",
                &works,
                |work| std::ops::ControlFlow::Continue(workspace::commit::execute(work, args)),
                |result| result.success,
            );
            print_batch(&report, args.format, |result| result.print(args.format))
        }
        Command::Delete(args) => {
            let works = selected_works(plans);
            let report = crate::batch::run(
                "delete",
                &works,
                |work| {
                    let result = args.execute(work);
                    if result.cancelled {
                        std::ops::ControlFlow::Break(result)
                    } else {
                        std::ops::ControlFlow::Continue(result)
                    }
                },
                |result| result.success,
            );
            print_batch(&report, args.format, |result| result.print(args.format))
        }
        Command::Run(_) | Command::Pr(_) => {
            let [plan]: [plan::Plan; 1] = plans.try_into().map_err(|_| {
                io::Error::other("run and pr require one plan; select its package scope first")
            })?;
            match &options.command {
                Command::Run(args) => advance_plan(args, plan),
                Command::Pr(args) => workspace::pr::run(args, &plan),
                _ => unreachable!("single-plan commands"),
            }
        }
    }
}

fn selected_works(plans: Vec<plan::Plan>) -> Vec<String> {
    plans
        .into_iter()
        .flat_map(|p| p.packages.into_iter().map(|t| t.work))
        .collect()
}

fn print_batch<R: Serialize>(
    report: &crate::batch::Report<'_, String, R>,
    format: ReportFormat,
    print: impl Fn(&R) -> io::Result<bool>,
) -> io::Result<bool> {
    match format {
        ReportFormat::Human => {
            for result in &report.results {
                print(result)?;
            }
            if !report.pending.is_empty() {
                crate::output_cli::stderr().message(
                    HumanLevel::Info,
                    None,
                    format_args!("pending: {}", report.pending.join(", ")),
                )?;
            }
        }
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), report)?,
    }
    Ok(report.success)
}

fn advance_plan(options: &RunOptions, plan: plan::Plan) -> io::Result<bool> {
    let workspace = workspace::discover()?;
    let root = workspace::directory(&workspace.configuration(), Path::new("tasks"), false)?;
    std::fs::create_dir_all(&root)?;
    // Observe pending work before new local builds can delay it.
    poll(&workspace, options, &root, &plan)?;
    let executable = std::env::current_exe()?;
    let mut tasks = Vec::new();
    for package in &plan.packages {
        let mut task = Task::new(package.work.clone(), options);
        match claim(&root, &package.work) {
            Err(error) => {
                task.phase = Phase::Paused;
                task.error = Some(format!("WORK is unavailable: {error}"));
            }
            Ok((path, _lock)) => {
                if path.join("task.toml").try_exists()? {
                    task = baseline::load(&path.join("task.toml"))?;
                    if task.work != package.work {
                        return Err(io::Error::other("task identifies another WORK"));
                    }
                }
                let result = (|| {
                    let settings = package.settings.inherit(&plan.defaults);
                    if let Some(receipt) = &options.check_receipt {
                        task.check_receipt = Some(std::path::absolute(receipt)?);
                    }
                    prepare(&workspace, options, &settings, &mut task)?;
                    verify_check(&workspace, &task)?;
                    advance(
                        &workspace,
                        &executable,
                        options,
                        &path,
                        &settings,
                        &mut task,
                    )?;
                    verify_check(&workspace, &task)
                })();
                if let Err(error) = result {
                    task.stop(&error);
                }
                baseline::save(&path.join("task.toml"), &task)?;
            }
        }
        tasks.push(task);
    }
    // Each completed round has its own selection. Never publish a partial round as an empty plan.
    let round = tempfile::Builder::new()
        .prefix("round-")
        .tempdir_in(&root)?
        .keep();
    let validated_plan = round.join("validated.plan.toml");
    let selected = plan
        .packages
        .into_iter()
        .filter(|p| {
            tasks
                .iter()
                .any(|t| t.work == p.work && t.phase == Phase::Validated)
        })
        .collect();
    baseline::save(
        &validated_plan,
        &plan::Plan {
            packages: selected,
            ..plan
        },
    )?;
    let report = Report {
        operation: "task",
        completed: true,
        observed_at: now(),
        validated_plan,
        tasks,
    };
    baseline::save(&round.join("report.toml"), &report)?;
    match options.format {
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
        ReportFormat::Human => {
            for task in &report.tasks {
                crate::output_cli::stderr().message(
                    match task.phase {
                        Phase::Failed => HumanLevel::Error,
                        Phase::Paused => HumanLevel::Warn,
                        _ => HumanLevel::Info,
                    },
                    Some(Path::new(&task.work)),
                    format_args!(
                        "task={:?}{}",
                        task.phase,
                        task.error
                            .as_ref()
                            .map_or(String::new(), |e| format!("; {e}"))
                    ),
                )?;
            }
            crate::output_cli::stderr().message(
                HumanLevel::Info,
                None,
                format_args!("Validated selection: {}", report.validated_plan.display()),
            )?;
        }
    }
    // A successful observation can still report a queued build.
    Ok(report
        .tasks
        .iter()
        .all(|t| !matches!(t.phase, Phase::Failed | Phase::Paused)))
}

fn verify_check(workspace: &workspace::Workspace, task: &Task) -> io::Result<()> {
    if let Some(receipt) = &task.check_receipt {
        let area = workspace.existing_development(&task.work)?;
        crate::check::directory::verify(receipt, &area)?;
    }
    Ok(())
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn snapshot(workspace: &workspace::Workspace, work: &str) -> io::Result<baseline::Files> {
    let area = workspace.existing_development(work)?;
    let mut files: baseline::Files = baseline::read(area.package_directory())?
        .into_iter()
        .map(|(name, file)| (format!("recipe/{name}"), file))
        .collect();
    for (name, file) in baseline::read(&area.sources())? {
        files.insert(format!("sources/{name}"), file);
    }
    let manifest = area.directory().join(format!("{}.toml", area.package()));
    if manifest.try_exists()? {
        files.insert(
            "authoring.toml".into(),
            baseline::File {
                sha256: crate::file_digest::read(&manifest)
                    .map_err(io::Error::other)?
                    .sha256,
                executable: crate::file_digest::executable(&std::fs::symlink_metadata(&manifest)?),
            },
        );
    }
    Ok(files)
}

fn validation_key(
    workspace: &workspace::Workspace,
    work: &str,
    remote: bool,
    settings: &plan::Settings,
) -> io::Result<String> {
    let config = workspace.configuration();
    let text = if remote {
        let area = workspace.existing_development(work)?;
        let mut text = toml::to_string(settings).map_err(io::Error::other)?;
        for path in [
            config.join("obs.toml"),
            area.directory().join("remote.toml"),
            area.directory().join("remote-receipt.toml"),
        ] {
            if path.try_exists()? {
                text.push('\n');
                text.push_str(&path.to_string_lossy());
                text.push_str(":\n");
                text.push_str(&crate::utf8_file::read(&path).map_err(io::Error::other)?);
            }
        }
        format!("obs\n{text}")
    } else {
        format!(
            "local\n{}",
            toml::to_string(&baseline::read(&config.join("build"))?).map_err(io::Error::other)?
        )
    };
    Ok(crate::utf8_file::sha256(&text))
}

fn prepare(
    workspace: &workspace::Workspace,
    options: &RunOptions,
    settings: &plan::Settings,
    task: &mut Task,
) -> io::Result<()> {
    if task.running {
        task.running = false;
        task.failed_step = Some(task.phase);
        task.phase = Phase::Paused;
        task.error =
            Some("Previous operation was interrupted. Inspect its logs before --retry.".into());
    }
    if matches!(task.phase, Phase::Failed | Phase::Paused) && !options.retry {
        return Ok(());
    }
    let remote = options
        .validation
        .map_or(task.remote, |v| v == Validation::Remote);
    if task.phase == Phase::Waiting && remote != task.remote {
        task.error =
            Some("The OBS build is pending. Repeat this request after it finishes.".into());
        return Ok(());
    }
    let changed = match &task.candidate {
        Some(files) => *files != snapshot(workspace, &task.work)?,
        None => false,
    };
    let key_changed = task.candidate.is_some()
        && task.validation_key != validation_key(workspace, &task.work, remote, settings)?;
    if changed
        || key_changed
        || options.retry && matches!(task.phase, Phase::Failed | Phase::Paused)
    {
        task.phase = Phase::Materials;
        task.remote = remote;
        task.polls = 0;
        task.query_errors = 0;
        task.next_poll = 0;
        task.failed_step = None;
        task.error = None;
        task.candidate = None;
    } else if task.phase == Phase::Validated && !task.remote {
        let area = workspace.existing_development(&task.work)?;
        let (changed, evidence) = validation::work(workspace, &area).map_err(io::Error::other)?;
        if !changed {
            task.phase = Phase::Unchanged;
        } else if !evidence.full_build_passed() {
            task.phase = Phase::Check;
        }
    }
    Ok(())
}

fn advance(
    workspace: &workspace::Workspace,
    executable: &Path,
    options: &RunOptions,
    root: &Path,
    settings: &plan::Settings,
    task: &mut Task,
) -> io::Result<()> {
    while matches!(
        task.phase,
        Phase::Materials | Phase::Check | Phase::Build | Phase::Submit
    ) {
        let phase = task.phase;
        if matches!(options.format, ReportFormat::Human) {
            crate::output_cli::stderr().message(
                HumanLevel::Info,
                Some(Path::new(&task.work)),
                format_args!("task={phase:?}"),
            )?;
        }
        if phase == Phase::Check {
            let area = workspace.existing_development(&task.work)?;
            let (changed, evidence) =
                validation::work(workspace, &area).map_err(io::Error::other)?;
            drop(area);
            let key = validation_key(workspace, &task.work, task.remote, settings)?;
            task.phase = if !changed {
                Phase::Unchanged
            } else if task.remote {
                Phase::Submit
            } else if key == task.validation_key && evidence.full_build_passed() {
                Phase::Validated
            } else {
                Phase::Build
            };
            if task.phase == Phase::Unchanged {
                task.validation_key = key;
            }
            baseline::save(&root.join("task.toml"), task)?;
            continue;
        }
        let args = match phase {
            Phase::Materials => vec!["source".into(), "fetch".into(), task.work.clone()],
            Phase::Build => vec![
                "build".into(),
                task.work.clone(),
                "--timeout".into(),
                options.timeout.to_string(),
            ],
            Phase::Submit => {
                let path = root.join("submission.plan.toml");
                baseline::save(
                    &path,
                    &plan::Plan {
                        defaults: settings.clone(),
                        packages: vec![plan::Task {
                            work: task.work.clone(),
                            settings: plan::Settings::default(),
                        }],
                        pr: None,
                    },
                )?;
                vec![
                    "task".into(),
                    "--plan".into(),
                    path.to_string_lossy().into_owned(),
                    "remote-build".into(),
                    "--fresh".into(),
                ]
            }
            _ => unreachable!("only executable phases reach command dispatch"),
        };
        let build_key = if phase == Phase::Build {
            Some(validation_key(workspace, &task.work, false, settings)?)
        } else {
            None
        };
        task.running = true;
        task.attempt += 1;
        baseline::save(&root.join("task.toml"), task)?;
        // Let build's own deadline expire before this process enforces a cleanup deadline.
        execution::run(
            executable,
            &args,
            root,
            task.attempt,
            if phase == Phase::Build {
                options.timeout.saturating_add(60)
            } else {
                options.timeout
            },
        )?;
        task.running = false;
        if let Some(before) = build_key
            && before != validation_key(workspace, &task.work, false, settings)?
        {
            return Err(io::Error::other(
                "Build configuration changed during validation.",
            ));
        }
        let after = snapshot(workspace, &task.work)?;
        if matches!(phase, Phase::Build | Phase::Submit) && task.candidate.as_ref() != Some(&after)
        {
            return Err(io::Error::other(
                "Package changed during validation. The result was not accepted.",
            ));
        }
        task.candidate = Some(after);
        task.phase = match phase {
            Phase::Materials => Phase::Check,
            Phase::Build => {
                let area = workspace.existing_development(&task.work)?;
                if !validation::work(workspace, &area)
                    .map_err(io::Error::other)?
                    .1
                    .full_build_passed()
                {
                    return Err(io::Error::other(
                        "No successful build matches the current package.",
                    ));
                }
                drop(area);
                task.validation_key = validation_key(workspace, &task.work, task.remote, settings)?;
                Phase::Validated
            }
            Phase::Submit => {
                task.validation_key = validation_key(workspace, &task.work, task.remote, settings)?;
                Phase::Waiting
            }
            _ => unreachable!("only executable phases reach command dispatch"),
        };
        baseline::save(&root.join("task.toml"), task)?;
    }
    Ok(())
}

fn poll(
    workspace: &workspace::Workspace,
    options: &RunOptions,
    root: &Path,
    plan: &plan::Plan,
) -> io::Result<()> {
    let mut pending = Vec::new();
    for package in &plan.packages {
        let Ok((path, lock)) = claim(root, &package.work) else {
            continue;
        };
        if !path.join("task.toml").try_exists()? {
            continue;
        }
        let task: Task = baseline::load(&path.join("task.toml"))?;
        if task.work != package.work {
            return Err(io::Error::other("task identifies another WORK"));
        }
        if task.remote
            && !task.running
            && (task.phase == Phase::Validated
                || task.phase == Phase::Waiting && (options.poll_now || task.next_poll <= now()))
        {
            pending.push((path, lock, task, package.settings.inherit(&plan.defaults)));
        }
    }
    if pending.is_empty() {
        return Ok(());
    }
    let works = pending
        .iter()
        .map(|(_, _, t, _)| t.work.clone())
        .collect::<Vec<_>>();
    let results = status::collect(workspace, &works);
    for (path, _lock, mut task, settings) in pending {
        let row = results
            .as_ref()
            .ok()
            .and_then(|r| r.tasks.iter().find(|r| r.work == task.work));
        let identity = snapshot(workspace, &task.work).and_then(|files| {
            Ok(task.candidate.as_ref() == Some(&files)
                && task.validation_key == validation_key(workspace, &task.work, true, &settings)?)
        });
        let unchanged = identity.as_ref().is_ok_and(|matches| *matches);
        let state = row.map_or(State::Unavailable, |r| r.state);
        task.polls = task.polls.saturating_add(1);
        task.query_errors = if state == State::Unavailable {
            task.query_errors.saturating_add(1)
        } else {
            0
        };
        task.phase = observed_phase(state, task.query_errors, unchanged);
        task.error = row
            .and_then(|r| r.error.clone())
            .or_else(|| results.as_ref().err().cloned());
        if !unchanged {
            task.error = Some(identity.err().map_or_else(
                || "Package or remote submission changed. Inspect it before --retry.".into(),
                |error| error.to_string(),
            ));
        } else if task.phase == Phase::Paused && task.error.is_none() {
            task.error = Some("Build status is unavailable. Polling stopped; inspect the submission before --retry.".into());
        }
        if matches!(task.phase, Phase::Failed | Phase::Paused) {
            task.failed_step = Some(Phase::Waiting);
        }
        let jitter = task.work.bytes().map(i64::from).sum::<i64>() % 6;
        task.next_poll = now() + (5_i64 << task.polls.min(6)).min(300) + jitter;
        if let Some(row) = row {
            baseline::save(&path.join("observation.toml"), row)?;
        }
        baseline::save(&path.join("task.toml"), &task)?;
    }
    Ok(())
}

fn observed_phase(state: State, errors: u32, unchanged: bool) -> Phase {
    if !unchanged {
        return Phase::Paused;
    }
    match state {
        State::Passed => Phase::Validated,
        State::Waiting => Phase::Waiting,
        State::Unavailable if errors < 20 => Phase::Waiting,
        State::Unavailable | State::Stale => Phase::Paused,
        State::Failed => Phase::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_builds_do_not_consume_query_error_budget() {
        for (state, errors, unchanged, expected) in [
            (State::Waiting, 200, true, Phase::Waiting),
            (State::Unavailable, 19, true, Phase::Waiting),
            (State::Unavailable, 20, true, Phase::Paused),
            (State::Passed, 20, true, Phase::Validated),
            (State::Passed, 0, false, Phase::Paused),
            (State::Failed, 0, true, Phase::Failed),
            (State::Stale, 0, true, Phase::Paused),
        ] {
            assert_eq!(observed_phase(state, errors, unchanged), expected);
        }
    }
}
