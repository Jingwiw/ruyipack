// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Publish an already committed package plan; never synthesize or squash commits.

mod github;
mod summary;
mod template;

use super::{baseline, git, invalid, recipe};
use crate::output_cli::{HumanLevel, ReportFormat};
use clap::Args;
use serde::Serialize;
use std::{
    collections::BTreeSet,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Args)]
#[command(
    after_help = "Default: preview only, without network access. Commit package changes first. --publish pushes the current branch without force, then creates a draft PR or updates its open PR. Closed PRs are not reopened. Uses Git and authenticated GitHub CLI (gh)."
)]
pub(crate) struct Options {
    /// Recipe repository; defaults to the workspace configuration.
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Add a package reminder to the PR summary; repeat for more packages.
    #[arg(long = "note", value_name = "PACKAGE: TEXT", value_parser = package_note)]
    notes: Vec<(String, String)>,
    /// Body template with `{{summary}}` and `{{obs_links}}` placeholders.
    #[arg(long)]
    template: Option<PathBuf>,
    /// Push commits and create or update the draft PR.
    #[arg(long)]
    publish: bool,
    /// Close the matching PR and cancel its unfinished GitHub Actions runs.
    #[arg(long, conflicts_with_all = ["publish", "notes", "template"])]
    close: bool,
    /// Deadline for each GitHub or push operation, in seconds.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

fn package_note(value: &str) -> Result<(String, String), String> {
    let (package, text) = value.split_once(':').ok_or("expected PACKAGE: TEXT")?;
    let (package, text) = (package.trim(), text.trim());
    template::line(package)
        .and_then(|()| template::line(text))
        .map_err(|error| error.to_string())?;
    Ok((package.to_owned(), text.to_owned()))
}

#[derive(Default, Serialize)]
struct Report {
    success: bool,
    published: bool,
    closed: bool,
    cancel_requested: Vec<u64>,
    finished_runs: Vec<u64>,
    pushed: bool,
    push_attempted: bool,
    stage: &'static str,
    target: String,
    branch: String,
    head: String,
    base: String,
    base_revision: String,
    title: String,
    body: String,
    works: Vec<String>,
    commits: Vec<String>,
    obs_links: BTreeSet<String>,
    warnings: Vec<String>,
    url: Option<String>,
    error: Option<String>,
}

pub(crate) fn run(options: &Options, plan: &crate::plan::Plan) -> io::Result<bool> {
    let mut report = Report {
        stage: "prepare",
        ..Report::default()
    };
    match perform(options, plan, &mut report) {
        Ok(()) => report.success = true,
        Err(error) => report.error = Some(error.to_string()),
    }
    match options.format {
        ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report)?,
        ReportFormat::Human => {
            let mut output = crate::output_cli::stderr();
            for warning in &report.warnings {
                output.message(HumanLevel::Warn, None, format_args!("{warning}"))?;
            }
            if report.pushed {
                output.message(
                    HumanLevel::Info,
                    None,
                    format_args!("pushed {}", report.head),
                )?;
            }
            if let Some(error) = &report.error {
                output.message(
                    HumanLevel::Error,
                    None,
                    format_args!("{}: {error}", report.stage),
                )?;
            } else if let Some(url) = &report.url {
                output.message(HumanLevel::Info, None, format_args!("{url}"))?;
            } else {
                writeln!(io::stdout().lock(), "{}\n\n{}", report.title, report.body)?;
                output.message(
                    HumanLevel::Info,
                    None,
                    format_args!(
                        "preview: {} WORKs, {} commits; no push or PR change",
                        report.works.len(),
                        report.commits.len()
                    ),
                )?;
            }
        }
    }
    Ok(report.success)
}

fn perform(options: &Options, plan: &crate::plan::Plan, report: &mut Report) -> io::Result<()> {
    let workspace = super::discover()?;
    let settings = plan
        .pr
        .as_ref()
        .ok_or_else(|| invalid("plan requires [pr] title and base"))?;
    template::line(&settings.title)?;
    let repo = fs_err::canonicalize(options.repo.as_deref().unwrap_or(workspace.recipes()))?;
    require_clean(&repo)?;
    let origin = github::repository(&git::line(&repo, &["remote", "get-url", "origin"])?)?;
    report.target = settings
        .target
        .as_deref()
        .map(github::repository)
        .transpose()?
        .unwrap_or_else(|| origin.clone());
    report.branch = git::line(&repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    report.head = git::line(&repo, &["rev-parse", "HEAD"])?;
    git::checked(&repo, ["check-ref-format", "--branch", &settings.base])?;
    if report.branch == settings.base {
        return Err(invalid("select a topic branch before preparing a PR"));
    }
    report.base.clone_from(&settings.base);
    report.base_revision = git::line(
        &repo,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{}^{{commit}}", settings.base),
        ],
    )?;
    git::checked(
        &repo,
        [
            "merge-base",
            "--is-ancestor",
            &report.base_revision,
            &report.head,
        ],
    )?;
    let range = format!("{}..{}", report.base_revision, report.head);
    if !git::line(&repo, &["rev-list", "--min-parents=2", &range])?.is_empty() {
        return Err(invalid(
            "PR range contains merge commits; select a linear package branch",
        ));
    }
    report.commits = git::line(&repo, &["rev-list", "--reverse", &range])?
        .lines()
        .map(str::to_owned)
        .collect();
    if report.commits.is_empty() {
        return Err(invalid("no commits to publish"));
    }
    if options.close {
        return github::close(&repo, &origin, options, report);
    }
    let exclusions = super::commit_scope::Exclusions::load(&workspace)?;
    let mut packages = BTreeSet::new();
    // Retain the WORK locks until preparation/publication has finished.
    let mut areas = Vec::new();
    for task in &plan.packages {
        let area = workspace.existing_development(&task.work)?;
        area.verify_binding()?;
        let package = workspace.specs.join(area.package());
        if !packages.insert(package.clone()) {
            return Err(invalid("multiple WORKs bind the same package"));
        }
        let mut files = baseline::read(area.package_directory())?;
        exclusions.filter(&mut files);
        let committed = recipe::files(&repo, &report.head, &package)?;
        exclusions.check(&committed)?;
        if files != committed {
            return Err(invalid(format!(
                "{}: WORK differs from committed package; commit or select the correct branch",
                task.work
            )));
        }
        if git::checked(
            &repo,
            [
                "diff",
                "--name-only",
                "-z",
                &report.base_revision,
                &report.head,
                "--",
                git::text(&package)?,
            ],
        )?
        .is_empty()
        {
            return Err(invalid(format!(
                "{}: no package diff against the base",
                task.work
            )));
        }
        template::obs_link(&area, &files, report)?;
        areas.push(area);
    }
    let mut touched = BTreeSet::new();
    let mut summary = summary::Summary::default();
    for commit in &report.commits {
        let bytes = git::checked(
            &repo,
            [
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-r",
                "-z",
                commit,
            ],
        )?;
        let mut scope = BTreeSet::new();
        for name in bytes.split(|b| *b == 0).filter(|name| !name.is_empty()) {
            let path = Path::new(std::str::from_utf8(name).map_err(io::Error::other)?);
            let package = packages
                .iter()
                .find(|pkg| path.starts_with(pkg))
                .ok_or_else(|| {
                    invalid(format!("{commit}: file outside plan: {}", path.display()))
                })?;
            scope.insert(package.clone());
        }
        if scope.len() != 1 {
            return Err(invalid(format!(
                "{commit}: each commit must change exactly one package"
            )));
        }
        let package = scope
            .first()
            .expect("one package")
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("invalid package path"))?;
        let message = git::line(
            &repo,
            &[
                "show",
                "-s",
                "--format=%s%n%(trailers:key=Action,valueonly)",
                commit,
            ],
        )?;
        summary.add(package, &message)?;
        touched.extend(scope);
    }
    if touched != packages {
        return Err(invalid(
            "plan contains packages with no commit in the selected range",
        ));
    }
    report.works = plan.packages.iter().map(|task| task.work.clone()).collect();
    report.title.clone_from(&settings.title);
    let text = template::load(
        options.template.as_deref().or(settings.template.as_deref()),
        &workspace.configuration().join("pr.md"),
        areas.iter().map(|area| area.directory().join("pr.md")),
    )?;
    for (package, text) in &options.notes {
        if !areas.iter().any(|area| area.package() == package) {
            return Err(invalid(format!(
                "{package}: PR note is outside the selected packages"
            )));
        }
        summary.note(package, text);
    }
    report.body = template::render(&text, &summary.render(), &report.obs_links)?;
    report.stage = "prepared";
    if options.publish {
        github::publish(&repo, &origin, options, report)?;
    }
    Ok(())
}

fn require_clean(repo: &Path) -> io::Result<()> {
    if !git::checked(repo, ["status", "--porcelain", "--untracked-files=all"])?.is_empty() {
        return Err(invalid("PR publication requires a clean repository"));
    }
    Ok(())
}
