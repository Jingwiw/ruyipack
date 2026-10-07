// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Deliver a package delta into a clean repository's current branch.

mod publication;
pub(crate) mod validation;

use super::{
    baseline::{self, Files},
    git, invalid,
};
use crate::output_cli::{HumanLevel, ReportFormat};
use clap::Args;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};

#[derive(Args)]
#[command(
    after_help = "Commits only package files to the repository's current branch. Requires a clean repository, except for an exact retry of this WORK's interrupted commit. Does not generate, download, build, switch branches, push or create a PR."
)]
pub(crate) struct Options {
    /// Development area containing the package changes.
    work: String,
    #[command(flatten)]
    arguments: Arguments,
}

#[derive(Args)]
pub(crate) struct Arguments {
    /// Override the recipe repository configured by init.
    #[arg(long, value_name = "PATH")]
    repo: Option<PathBuf>,
    /// Commit only the bound SPEC; leave other package changes in WORK.
    #[arg(long)]
    spec_only: bool,
    /// Require a successful full local build with matching package inputs.
    #[arg(long)]
    require_build: bool,
    /// Show changes without modifying the repository.
    #[arg(long)]
    dry_run: bool,
    /// Use this message instead of a fact-based package summary.
    #[arg(short, long)]
    message: Option<String>,
    /// Describe each action separately; join the final action with "and".
    #[arg(long = "action", conflicts_with = "message")]
    actions: Vec<String>,
    /// Git commit deadline in seconds, including hooks and signing.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

#[derive(Serialize)]
pub(crate) struct Report {
    operation: &'static str,
    work: String,
    preview: bool,
    pub(crate) success: bool,
    repository: Option<PathBuf>,
    branch: Option<String>,
    message: Option<String>,
    changes: Vec<String>,
    diff: String,
    commit: Option<String>,
    retained: bool,
    admission: Option<validation::Admission>,
    build: Option<crate::build::evidence::Evidence>,
    error: Option<String>,
}

// This is an in-flight recovery record, not another editable recipe. Before and
// after identities let retries reject unrelated user edits and hook mutations.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    repository: PathBuf,
    branch: String,
    parent: String,
    package: String,
    message: String,
    before: Files,
    after: Files,
    baseline: baseline::Baseline,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    execute(&options.work, &options.arguments).print(options.arguments.format)
}

/// Commit one WORK and retain its exact result, including partial publication.
pub(crate) fn execute(work: &str, options: &Arguments) -> Report {
    let mut report = Report {
        operation: "commit",
        work: work.to_owned(),
        preview: options.dry_run,
        success: false,
        repository: None,
        branch: None,
        message: None,
        changes: vec![],
        diff: String::new(),
        commit: None,
        retained: false,
        admission: None,
        build: None,
        error: None,
    };
    match perform(options, &mut report) {
        Ok(()) => report.success = true,
        Err(error) => report.error = Some(error.to_string()),
    }
    report
}

impl Report {
    pub(crate) fn print(&self, format: ReportFormat) -> io::Result<bool> {
        match format {
            ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), self)?,
            ReportFormat::Human => {
                let mut out = crate::output_cli::stderr();
                if let Some(branch) = &self.branch {
                    out.message(
                        HumanLevel::Info,
                        Some(Path::new(&self.work)),
                        format_args!("branch={branch}; files={}", self.changes.len()),
                    )?;
                }
                if self.preview
                    && let Some(error) = self.admission.as_ref().and_then(|a| a.error.as_ref())
                {
                    out.message(
                        HumanLevel::Warn,
                        Some(Path::new(&self.work)),
                        format_args!("submit: {error}"),
                    )?;
                }
                if let Some(warning) = self
                    .admission
                    .as_ref()
                    .and_then(|a| a.name_warning.as_ref())
                {
                    out.message(
                        HumanLevel::Warn,
                        Some(Path::new(&self.work)),
                        format_args!(
                            "directory={}; SPEC Name={}",
                            warning.directory,
                            warning
                                .literal
                                .as_deref()
                                .unwrap_or("not statically determined")
                        ),
                    )?;
                }
                if let Some(spec) = self
                    .admission
                    .as_ref()
                    .and_then(|a| a.spec_fallback.as_ref())
                {
                    out.message(
                        HumanLevel::Warn,
                        Some(Path::new(&self.work)),
                        format_args!("selected SPEC={spec}; unique-file fallback"),
                    )?;
                }
                if let Some(build) = &self.build {
                    out.message(
                        if matches!(build.status, "failed" | "stale" | "unavailable") {
                            HumanLevel::Warn
                        } else {
                            HumanLevel::Info
                        },
                        Some(Path::new(&self.work)),
                        format_args!("build={}; stage={}", build.status, build.stage_name()),
                    )?;
                }
                if self.preview {
                    use std::io::Write;
                    io::stdout().lock().write_all(self.diff.as_bytes())?;
                }
                if self.error.is_some()
                    && let Some(id) = &self.commit
                {
                    out.message(
                        HumanLevel::Info,
                        Some(Path::new(&self.work)),
                        format_args!(
                            "commit created: {id}; completion failed; recovery state retained"
                        ),
                    )?;
                }
                let text = self.error.as_deref().map_or_else(
                    || {
                        self.commit.as_ref().map_or_else(
                            || {
                                if self.preview {
                                    "preview; repository unchanged".into()
                                } else {
                                    "no changes to commit".into()
                                }
                            },
                            |id| format!("committed {id}"),
                        )
                    },
                    str::to_owned,
                );
                out.message(
                    if self.success {
                        HumanLevel::Info
                    } else {
                        HumanLevel::Error
                    },
                    Some(Path::new(&self.work)),
                    format_args!("{text}"),
                )?;
            }
        }
        Ok(self.success)
    }
}

fn perform(options: &Arguments, report: &mut Report) -> io::Result<()> {
    let workspace = super::discover()?;
    let area = workspace.existing_development(&report.work)?;
    area.verify_binding()?;
    let repository = fs::canonicalize(options.repo.as_deref().unwrap_or(&workspace.recipes))?;
    let root = git::line(&repository, &["rev-parse", "--show-toplevel"])?;
    if fs::canonicalize(root)? != repository {
        return Err(invalid("--repo must name the Git repository root"));
    }
    git::require_personal_origin(&repository)?;
    let branch = git::line(&repository, &["symbolic-ref", "--quiet", "HEAD"])?;
    let parent = git::line(&repository, &["rev-parse", "HEAD^{commit}"])?;
    let common = git::line(
        &repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let _lock = if options.dry_run {
        None
    } else {
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(Path::new(&common).join("ruyipack-commit.lock"))?;
        Some(crate::file_lock::FileLock::try_lock(lock.into_file()).map_err(io::Error::other)?)
    };
    report.repository = Some(repository.clone());
    report.branch = Some(branch.clone());
    let package = workspace.specs.join(area.package());
    let package = git::text(&package)?.to_owned();
    baseline::relative(&package)?;
    let target = super::directory(&repository, Path::new(&package), false)?;
    let exclusions = super::commit_scope::Exclusions::load(&workspace)?;
    exclusions.check(&super::recipe::files(
        &repository,
        "HEAD",
        Path::new(&package),
    )?)?;
    let mut current = baseline::read(area.package_directory())?;
    exclusions.filter(&mut current);
    let selected_spec =
        super::recipe::select_spec(Some(area.package()), current.keys().map(Path::new))?;
    let state = area.directory().join("commit.toml");
    let pending: Pending = if state.try_exists()? {
        report.retained = true;
        let pending: Pending = baseline::load(&state)?;
        if pending.repository != repository
            || pending.branch != branch
            || pending.package != package
        {
            return Err(invalid(
                "pending commit belongs to another repository or branch; restore that selection before retrying",
            ));
        }
        if options
            .message
            .as_ref()
            .is_some_and(|message| message != &pending.message)
            || (!options.actions.is_empty()
                && action_message(area.package(), &options.actions)? != pending.message)
            || options.spec_only
                && changes(&pending.before, &pending.after)
                    .iter()
                    .any(|name| Path::new(name) != selected_spec)
        {
            return Err(invalid(
                "retry must keep the pending commit message and file scope",
            ));
        }
        pending
    } else {
        publication::require_clean(&repository)?;
        if super::recipe::files(&repository, "HEAD", Path::new(&package))?
            != baseline::read(&target)?
        {
            return Err(invalid(
                "target package differs from committed HEAD, including ignored or hidden changes",
            ));
        }
        let mut base: baseline::Baseline = baseline::load(&area.directory().join("baseline.toml"))?;
        exclusions.filter(&mut base.files);
        plan(
            &target,
            area.package_directory(),
            area.package(),
            &base,
            &current,
            options,
            Pending {
                repository,
                branch,
                parent,
                package,
                message: String::new(),
                before: baseline::read(&target)?,
                after: Files::new(),
                baseline: baseline::Baseline {
                    files: Files::new(),
                    allow_create: false,
                },
            },
        )?
    };
    exclusions.check(&pending.before)?;
    exclusions.check(&pending.after)?;
    let changed = changes(&pending.before, &pending.after);
    report.changes.clone_from(&changed);
    report.message = Some(pending.message.clone());
    let (admission, build) = validation::assess(&pending, &area);
    report.admission = Some(admission);
    report.build = build;
    if changed.is_empty() {
        if !options.dry_run {
            baseline::save(&area.directory().join("baseline.toml"), &pending.baseline)?;
        }
        return Ok(());
    }
    if options.require_build
        && !report
            .build
            .as_ref()
            .is_some_and(crate::build::evidence::Evidence::full_build_passed)
        && let Some(admission) = report.admission.as_mut()
        && admission.allowed
    {
        admission.allowed = false;
        admission.error = Some(
            "matching successful full local build required; run build WORK before committing"
                .into(),
        );
    }
    for name in &changed {
        baseline::relative(name)?;
    }
    if options.dry_run {
        publication::verify_work(&pending, &area)?;
        report.diff = preview(&pending, area.package_directory(), &changed)?;
        if options.require_build
            && let Some(error) = report.admission.as_ref().and_then(|a| a.error.as_ref())
        {
            return Err(invalid(error.clone()));
        }
        return Ok(());
    }
    publication::execute(
        &pending,
        &area,
        &target,
        &state,
        std::time::Duration::from_secs(options.timeout),
        report,
    )
}

fn plan(
    target: &Path,
    source: &Path,
    package: &str,
    base: &baseline::Baseline,
    current: &Files,
    options: &Arguments,
    mut pending: Pending,
) -> io::Result<Pending> {
    pending.after = pending.before.clone();
    pending.baseline.files = base.files.clone();
    let selected = super::recipe::select_spec(Some(package), current.keys().map(Path::new))?;
    let spec = git::text(&selected)?.to_owned();
    for name in base
        .files
        .keys()
        .chain(current.keys())
        .collect::<BTreeSet<_>>()
    {
        baseline::relative(name)?;
        if options.spec_only && name != &spec {
            continue;
        }
        let (old, new, destination) = (
            base.files.get(name),
            current.get(name),
            pending.before.get(name),
        );
        if !(base.allow_create && pending.before.is_empty())
            && destination != old
            && destination != new
        {
            return Err(invalid(format!(
                "{name}: repository differs from both import baseline and WORK; resolve the conflict before committing"
            )));
        }
        if let Some(file) = new {
            pending.after.insert(name.clone(), file.clone());
            pending.baseline.files.insert(name.clone(), file.clone());
        } else {
            pending.after.remove(name);
            pending.baseline.files.remove(name);
        }
    }
    let content = crate::utf8_file::read(&target.join(&spec)).ok();
    pending.message = if let Some(message) = &options.message {
        if message.trim().is_empty() {
            return Err(invalid("commit message must not be empty"));
        }
        message.clone()
    } else if !options.actions.is_empty() {
        action_message(package, &options.actions)?
    } else {
        let old = content
            .as_deref()
            .and_then(|text| field(&crate::spec::ParsedSpec::parse(text), "version"));
        let text = crate::utf8_file::read(&source.join(&spec)).ok();
        let new = text
            .as_deref()
            .and_then(|text| field(&crate::spec::ParsedSpec::parse(text), "version"));
        // Message generation never decides what is included in the commit.
        match (content.is_none(), old, new) {
            (true, _, Some(new)) => format!("SPECS: {package}: Add package at {new}"),
            (false, Some(old), Some(new)) if old != new => {
                let version_only =
                    content
                        .as_deref()
                        .zip(text.as_deref())
                        .is_some_and(|(before, after)| {
                            let parsed = crate::spec::ParsedSpec::parse(before);
                            let Ok(snapshot) = crate::spec::document::Snapshot::capture_selected(
                                &parsed,
                                &["package.version".into()],
                            ) else {
                                return false;
                            };
                            let mut document = snapshot.document().clone();
                            document["package"]["version"] = toml::Value::String(new.clone());
                            snapshot
                                .render(&document, &[])
                                .is_ok_and(|rendered| rendered.source() == after)
                        });
                if version_only && changes(&pending.before, &pending.after).len() == 1 {
                    format!("SPECS: {package}: Update {old} -> {new}")
                } else {
                    format!("SPECS: {package}: Update {old} -> {new} and update packaging")
                }
            }
            (true, _, _) => format!("SPECS: {package}: Add package"),
            _ => format!("SPECS: {package}: Update packaging"),
        }
    };
    Ok(pending)
}

fn action_message(package: &str, actions: &[String]) -> io::Result<String> {
    if actions
        .iter()
        .any(|action| action.trim().is_empty() || action.chars().any(char::is_control))
    {
        return Err(invalid("commit actions must be nonempty single lines"));
    }
    let (last, rest) = actions
        .split_last()
        .ok_or_else(|| invalid("no commit action"))?;
    let summary = if rest.is_empty() {
        last.clone()
    } else {
        format!("{} and {last}", rest.join(", "))
    };
    let trailers = actions
        .iter()
        .map(|action| format!("Action: {action}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("SPECS: {package}: {summary}\n\n{trailers}"))
}

fn field(parsed: &crate::spec::ParsedSpec<'_>, name: &str) -> Option<String> {
    let snapshot =
        crate::spec::document::Snapshot::capture_selected(parsed, &[format!("package.{name}")])
            .ok()?;
    snapshot
        .document()
        .get("package")?
        .get(name)?
        .as_str()
        .filter(|v| !v.contains('%'))
        .map(str::to_owned)
}

fn changes(before: &Files, after: &Files) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

fn preview(pending: &Pending, source: &Path, changed: &[String]) -> io::Result<String> {
    let (before_files, after) = (&pending.before, &pending.after);
    let mut result = String::new();
    for name in changed {
        use std::fmt::Write;
        let mode = |file: &baseline::File| if file.executable { "100755" } else { "100644" };
        let _ = writeln!(
            result,
            "diff --git {:?} {:?}",
            format!("a/{name}"),
            format!("b/{name}")
        );
        match (before_files.get(name), after.get(name)) {
            (None, Some(new)) => {
                let _ = writeln!(result, "new file mode {}", mode(new));
            }
            (Some(old), None) => {
                let _ = writeln!(result, "deleted file mode {}", mode(old));
            }
            (Some(old), Some(new)) if old.executable != new.executable => {
                let _ = writeln!(result, "old mode {}\nnew mode {}", mode(old), mode(new));
            }
            _ => {}
        }
        // A failed attempt may already have changed the working tree. Diff against
        // the planned parent, not that partially published directory.
        let before = if before_files.contains_key(name) {
            git::checked(
                &pending.repository,
                [
                    "show",
                    &format!("{}:{}/{name}", pending.parent, pending.package),
                ],
            )?
        } else {
            vec![]
        };
        let new = if after.contains_key(name) {
            fs::read(source.join(name))?
        } else {
            vec![]
        };
        if let (Ok(old), Ok(new)) = (std::str::from_utf8(&before), std::str::from_utf8(&new)) {
            result.push_str(
                &similar::TextDiff::from_lines(old, new)
                    .unified_diff()
                    .header(&format!("a/{name}"), &format!("b/{name}"))
                    .to_string(),
            );
        } else {
            let _ = writeln!(result, "Binary file changed: {name}");
        }
    }
    Ok(result)
}

#[cfg(test)]
mod message_tests {
    use super::action_message;
    #[test]
    fn action_lists_use_commas_and_one_final_conjunction() {
        for (actions, expected) in [
            (vec!["Update version"], "Update version"),
            (
                vec!["Update version", "refresh hashes"],
                "Update version and refresh hashes",
            ),
            (
                vec!["Update version", "refresh hashes", "remove signatures"],
                "Update version, refresh hashes and remove signatures",
            ),
        ] {
            let actions = actions.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(
                action_message("pkg", &actions)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap(),
                format!("SPECS: pkg: {expected}")
            );
        }
        assert!(action_message("pkg", &["bad\nmessage".into()]).is_err());
    }
}
