// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! A bounded maintenance round. Commands own operations; tasks own progress.
mod execution;

use crate::{
    file_lock::FileLock,
    output_cli::{HumanLevel, ReportFormat},
    plan,
    workspace::{self, baseline},
};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub(crate) struct Options {
    /// Package selection and OBS defaults; the input plan is never changed.
    #[arg(long)]
    plan: PathBuf,
    /// Include supported patch-free upgrades in the initial repair.
    #[arg(long)]
    upgrade: bool,
    /// Use OBS instead of a local build; each invocation collects due results once.
    #[arg(long)]
    remote: bool,
    /// Retry failed or interrupted tasks after a manual repair. Does not rerun auto-fix.
    #[arg(long)]
    retry_failed: bool,
    /// Observe pending OBS work now instead of waiting for the next scheduled poll.
    #[arg(long, requires = "remote")]
    poll_now: bool,
    /// Deadline for each command, including child cleanup.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Fix,
    Materials,
    Check,
    Preview,
    Build,
    Submit,
    Waiting,
    Ready,
    Unchanged,
    Failed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Task {
    format_version: u32,
    work: String,
    intent: String,
    phase: Phase,
    running: bool,
    attempt: u64,
    next_poll: i64,
    polls: u32,
    candidate: Option<baseline::Files>,
    error: Option<String>,
    failed_step: Option<Phase>,
}

#[derive(Serialize)]
struct Report {
    format_version: u32,
    operation: &'static str,
    success: bool,
    ready_plan: PathBuf,
    tasks: Vec<Task>,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    let workspace = workspace::discover()?;
    let plan: plan::Plan = baseline::load(&options.plan)?;
    let mut seen = BTreeSet::new();
    if plan.packages.is_empty() {
        return Err(io::Error::other("maintenance plan is empty"));
    }
    for task in &plan.packages {
        crate::check::metadata::Field::Name
            .validate_at(&task.work, "WORK")
            .map_err(io::Error::other)?;
        if !seen.insert(&task.work) {
            return Err(io::Error::other(format!("duplicate WORK: {}", task.work)));
        }
    }
    let root = workspace::directory(&workspace.configuration(), Path::new("maintenance"), false)?;
    std::fs::create_dir_all(&root)?;
    let plan_id = crate::utf8_file::sha256(&toml::to_string(&plan).map_err(io::Error::other)?);
    let _plan_lock = FileLock::try_lock(
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(format!("{plan_id}.lock")))?,
    )
    .map_err(io::Error::other)?;
    let ready_plan = root.join(format!("ready-{plan_id}.toml"));
    // Never leave the previous selection available after an interrupted refresh.
    baseline::save(
        &ready_plan,
        &plan::Plan {
            defaults: plan.defaults.clone(),
            packages: Vec::new(),
            pr: plan.pr.clone(),
        },
    )?;
    let executable = std::env::current_exe()?;
    let mut reports = Vec::new();
    let mut due = Vec::new();
    let mut locks = Vec::new();
    for package in &plan.packages {
        let path = workspace::directory(&root, Path::new(&package.work), false)?;
        std::fs::create_dir_all(&path)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.join("lock"))?;
        let lock = FileLock::try_lock(file).map_err(io::Error::other)?;
        let mut settings = package.settings.inherit(&plan.defaults);
        // The submission command owns resolution against workspace OBS defaults.
        let intent = crate::utf8_file::sha256(&format!(
            "{}\n{}\n{}",
            options.upgrade,
            options.remote,
            toml::to_string(&settings).map_err(io::Error::other)?
        ));
        let state_path = path.join("task.toml");
        let mut task: Task = if state_path.try_exists()? {
            baseline::load(&state_path)?
        } else {
            Task {
                format_version: 1,
                work: package.work.clone(),
                intent: intent.clone(),
                phase: Phase::Fix,
                running: false,
                attempt: 0,
                next_poll: 0,
                polls: 0,
                candidate: None,
                error: None,
                failed_step: None,
            }
        };
        if task.format_version != 1 || task.work != package.work || task.intent != intent {
            return Err(io::Error::other(format!(
                "{}: task intent differs; use a separate WORK",
                package.work
            )));
        }
        if task.running {
            task.running = false;
            task.failed_step = Some(task.phase);
            task.phase = Phase::Failed;
            task.error = Some(
                "previous command was interrupted; inspect retained logs before --retry-failed"
                    .into(),
            );
        }
        if options.retry_failed && task.phase == Phase::Failed {
            task.phase = Phase::Materials;
            task.running = false;
            task.error = None;
            task.failed_step = None;
            task.candidate = None;
        }
        if let Err(error) = advance(
            &workspace,
            &executable,
            options,
            &path,
            &mut task,
            &mut settings,
        ) {
            task.failed_step = Some(task.phase);
            task.phase = Phase::Failed;
            task.running = false;
            task.error = Some(error.to_string());
        }
        baseline::save(&state_path, &task)?;
        if task.phase == Phase::Waiting && (options.poll_now || task.next_poll <= now()) {
            due.push(plan::Task {
                work: task.work.clone(),
                settings,
            });
        }
        reports.push(task);
        locks.push(lock);
    }
    if !due.is_empty() {
        poll(&workspace, &executable, options, &root, &due, &mut reports)?;
    }
    let ready: Vec<_> = plan
        .packages
        .into_iter()
        .filter(|item| {
            reports
                .iter()
                .any(|task| task.work == item.work && task.phase == Phase::Ready)
        })
        .collect();
    baseline::save(
        &ready_plan,
        &plan::Plan {
            packages: ready,
            ..plan
        },
    )?;
    let report = Report {
        format_version: 1,
        operation: "maintain",
        success: reports
            .iter()
            .all(|t| matches!(t.phase, Phase::Ready | Phase::Unchanged)),
        ready_plan,
        tasks: reports,
    };
    match options.format {
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
        ReportFormat::Human => {
            for task in &report.tasks {
                crate::output_cli::stderr().message(
                    if task.phase == Phase::Failed {
                        HumanLevel::Error
                    } else {
                        HumanLevel::Info
                    },
                    Some(Path::new(&task.work)),
                    format_args!(
                        "maintenance={:?}{}",
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
                format_args!("ready plan: {}", report.ready_plan.display()),
            )?;
        }
    }
    drop(locks);
    Ok(report.success)
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
    // Prefixes cannot overlap recipe-relative paths: those are validated separately.
    for (name, file) in baseline::read(&area.sources())? {
        files.insert(format!("sources/{name}"), file);
    }
    for (name, file) in baseline::read(&workspace.configuration().join("build"))? {
        files.insert(format!("environment/{name}"), file);
    }
    let manifest = area.directory().join(format!("{}.toml", area.package()));
    if manifest.try_exists()? {
        let meta = std::fs::symlink_metadata(&manifest)?;
        files.insert(
            "authoring.toml".into(),
            baseline::File {
                sha256: crate::file_digest::read(&manifest)
                    .map_err(io::Error::other)?
                    .sha256,
                executable: crate::file_digest::executable(&meta),
            },
        );
    }
    Ok(files)
}

fn advance(
    workspace: &workspace::Workspace,
    executable: &Path,
    options: &Options,
    root: &Path,
    task: &mut Task,
    settings: &mut plan::Settings,
) -> io::Result<()> {
    if task.phase == Phase::Failed {
        return Ok(());
    }
    if let Some(candidate) = &task.candidate
        && *candidate != snapshot(workspace, &task.work)?
    {
        return Err(io::Error::other(
            "candidate or build environment changed; inspect and use --retry-failed",
        ));
    }
    while !matches!(task.phase, Phase::Ready | Phase::Unchanged | Phase::Waiting) {
        let phase = task.phase;
        if matches!(options.format, ReportFormat::Human) {
            crate::output_cli::stderr().message(
                HumanLevel::Info,
                Some(Path::new(&task.work)),
                format_args!("maintenance={phase:?}"),
            )?;
        }
        let work = task.work.clone();
        let mut args: Vec<String> = match phase {
            Phase::Fix => vec!["check".into(), work.clone(), "--auto-fix".into()],
            Phase::Materials => vec!["source".into(), "fetch".into(), work.clone()],
            Phase::Check => vec![
                "check".into(),
                work.clone(),
                "--policy".into(),
                "submit".into(),
                "--materials".into(),
            ],
            Phase::Preview => vec!["commit".into(), work.clone(), "--dry-run".into()],
            Phase::Build => vec![
                "build".into(),
                work.clone(),
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
                            work: work.clone(),
                            settings: plan::Settings::default(),
                        }],
                        pr: None,
                    },
                )?;
                vec![
                    "remote-build".into(),
                    "--plan".into(),
                    path.to_string_lossy().into_owned(),
                    "--fresh".into(),
                ]
            }
            _ => unreachable!("terminal phases are excluded above"),
        };
        if phase == Phase::Fix && options.upgrade {
            args.push("--upgrade".into());
        }
        task.running = true;
        task.attempt += 1;
        baseline::save(&root.join("task.toml"), task)?;
        let result = execution::run(executable, &args, root, task.attempt, options.timeout)?;
        task.running = false;
        let after = snapshot(workspace, &work)?;
        if !matches!(phase, Phase::Fix | Phase::Materials)
            && task.candidate.as_ref() != Some(&after)
        {
            return Err(io::Error::other(
                "candidate changed during validation; results were not admitted",
            ));
        }
        task.candidate = Some(after);
        task.phase = match phase {
            Phase::Fix => Phase::Materials,
            Phase::Materials => Phase::Check,
            Phase::Check => Phase::Preview,
            Phase::Preview => {
                if result
                    .get("admission")
                    .and_then(|v| v.get("allowed"))
                    .and_then(toml::Value::as_bool)
                    != Some(true)
                {
                    return Err(io::Error::other(
                        "commit admission failed; inspect command report",
                    ));
                }
                if result
                    .get("changes")
                    .and_then(toml::Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    Phase::Unchanged
                } else if options.remote {
                    Phase::Submit
                } else {
                    Phase::Build
                }
            }
            Phase::Build => {
                task.attempt += 1;
                task.running = true;
                baseline::save(&root.join("task.toml"), task)?;
                let gate = execution::run(
                    executable,
                    &[
                        "commit".into(),
                        work,
                        "--dry-run".into(),
                        "--require-build".into(),
                    ],
                    root,
                    task.attempt,
                    options.timeout,
                )?;
                task.running = false;
                if task.candidate.as_ref() != Some(&snapshot(workspace, &task.work)?) {
                    return Err(io::Error::other("candidate changed during build admission"));
                }
                if gate
                    .get("admission")
                    .and_then(|v| v.get("allowed"))
                    .and_then(toml::Value::as_bool)
                    != Some(true)
                    || gate
                        .get("build")
                        .and_then(|v| v.get("status"))
                        .and_then(toml::Value::as_str)
                        != Some("passed")
                {
                    return Err(io::Error::other(
                        "local build evidence does not admit this delivery",
                    ));
                }
                Phase::Ready
            }
            Phase::Submit => Phase::Waiting,
            _ => unreachable!("terminal phases are excluded above"),
        };
        baseline::save(&root.join("task.toml"), task)?;
    }
    Ok(())
}

fn poll(
    workspace: &workspace::Workspace,
    executable: &Path,
    options: &Options,
    root: &Path,
    due: &[plan::Task],
    reports: &mut [Task],
) -> io::Result<()> {
    // A separate invocation owns this temporary plan, so parallel disjoint plans cannot overwrite it.
    let directory = tempfile::Builder::new()
        .prefix("poll-")
        .tempdir_in(root)?
        .keep();
    let path = directory.join("poll.toml");
    baseline::save(
        &path,
        &plan::Plan {
            defaults: plan::Settings::default(),
            packages: due.to_vec(),
            pr: None,
        },
    )?;
    let result = execution::observe(
        executable,
        &[
            "remote-build".into(),
            "--status".into(),
            "--plan".into(),
            path.to_string_lossy().into_owned(),
        ],
        &directory,
        0,
        options.timeout,
    );
    for task in reports
        .iter_mut()
        .filter(|t| due.iter().any(|d| d.work == t.work))
    {
        let observed = result
            .as_ref()
            .ok()
            .and_then(|r| r.get("tasks"))
            .and_then(toml::Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|r| r.get("work").and_then(toml::Value::as_str) == Some(&task.work))
            });
        task.polls = task.polls.saturating_add(1);
        let state = observed
            .and_then(|r| r.get("state"))
            .and_then(toml::Value::as_str)
            .unwrap_or("unavailable");
        let unchanged =
            state != "passed" || task.candidate.as_ref() == Some(&snapshot(workspace, &task.work)?);
        task.phase = observed_phase(state, task.polls, unchanged);
        task.error = observed
            .and_then(|r| r.get("error"))
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
            .or_else(|| result.as_ref().err().map(ToString::to_string));
        if task.phase == Phase::Failed && task.error.is_none() {
            task.error = Some(format!("remote-build={state}; inspect remote evidence"));
        }
        if task.phase == Phase::Failed {
            task.failed_step = Some(Phase::Waiting);
        }
        // Spread package polls without a random runtime dependency.
        let jitter = task.work.bytes().map(i64::from).sum::<i64>() % 6;
        task.next_poll = now() + (5_i64 << task.polls.min(6)).min(300) + jitter;
        let work_root = root.join(&task.work);
        if let Some(row) = observed {
            baseline::save(&work_root.join("observation.toml"), row)?;
        }
        baseline::save(&work_root.join("task.toml"), task)?;
    }
    Ok(())
}

fn observed_phase(state: &str, polls: u32, unchanged: bool) -> Phase {
    match state {
        "passed" if unchanged => Phase::Ready,
        "waiting" | "unavailable" if polls < 20 => Phase::Waiting,
        _ => Phase::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::{Phase, observed_phase};

    #[test]
    fn only_current_success_passes_and_waiting_has_a_budget() {
        for (state, polls, unchanged, expected) in [
            ("waiting", 1, true, Phase::Waiting),
            ("unavailable", 19, true, Phase::Waiting),
            ("waiting", 20, true, Phase::Failed),
            ("unavailable", 20, true, Phase::Failed),
            ("passed", 20, true, Phase::Ready),
            ("passed", 1, false, Phase::Failed),
            ("failed", 1, true, Phase::Failed),
            ("stale", 1, true, Phase::Failed),
            ("unknown", 1, true, Phase::Failed),
        ] {
            assert_eq!(observed_phase(state, polls, unchanged), expected, "{state}");
        }
    }
}
