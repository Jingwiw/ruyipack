// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Preserve an exact interrupted publication; never reset the user's repository.

use super::{Pending, Report, baseline, changes, git, invalid};
use fs_err as fs;
use std::{
    io::{self, Write},
    path::Path,
};

pub(super) fn require_clean(repo: &Path) -> io::Result<()> {
    if !git::checked(
        repo,
        [
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?
    .is_empty()
    {
        return Err(invalid(
            "target repository must have a clean worktree and index; commit or move existing changes first",
        ));
    }
    Ok(())
}

pub(super) fn execute(
    pending: &Pending,
    area: &crate::workspace::Development,
    target: &Path,
    state: &Path,
    timeout: std::time::Duration,
    report: &mut Report,
) -> io::Result<()> {
    let head = git::line(&pending.repository, &["rev-parse", "HEAD"])?;
    let changed = changes(&pending.before, &pending.after);
    if head != pending.parent {
        // A commit may succeed before writing WORK's baseline. Recover only its
        // exact parent and complete tree, not merely a matching package version.
        require_clean(&pending.repository)?;
        let parents = git::line(&pending.repository, &["show", "-s", "--format=%P", "HEAD"])?;
        if parents != pending.parent
            || baseline::read(target)? != pending.after
            || super::super::recipe::files(
                &pending.repository,
                "HEAD",
                Path::new(&pending.package),
            )? != pending.after
            || committed_paths(pending)? != changed
        {
            return Err(invalid(
                "repository HEAD changed; pending commit retained for review",
            ));
        }
        return finish(pending, area, state, &head, report);
    }
    verify_work(pending, area)?;
    if let Some(error) = report.admission.as_ref().and_then(|a| a.error.as_ref()) {
        return Err(invalid(error.clone()));
    }
    // Before any write, reject dirty paths not owned by this recovery plan.
    check_status(pending, &changed)?;
    let actual = baseline::read(target)?;
    for name in actual
        .keys()
        .chain(pending.before.keys())
        .chain(pending.after.keys())
    {
        let value = actual.get(name);
        if value != pending.before.get(name) && value != pending.after.get(name) {
            return Err(invalid(format!(
                "{name}: repository changed during publication; pending commit retained"
            )));
        }
    }
    baseline::save(state, pending)?;
    report.retained = true;
    apply_files(pending, area.package_directory(), target, &changed)?;
    if baseline::read(target)? != pending.after {
        return Err(invalid(
            "package changed during publication; changes retained",
        ));
    }
    verify_head(pending)?;
    check_status(pending, &changed)?;
    let paths: Vec<_> = changed
        .iter()
        .map(|name| format!("{}/{name}", pending.package))
        .collect();
    let mut args = vec!["add", "--all", "--"];
    args.extend(paths.iter().map(String::as_str));
    git::checked(&pending.repository, args)?;
    verify_work(pending, area)?;
    verify_head(pending)?;
    check_status(pending, &changed)?;
    let mut args = vec![
        "commit",
        "--only",
        "--signoff",
        "-m",
        &pending.message,
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    if let Err(error) = git::checked_with_budget(&pending.repository, args, timeout) {
        if let Ok(head) = git::line(&pending.repository, &["rev-parse", "HEAD"])
            && head != pending.parent
        {
            report.commit = Some(head);
        }
        return Err(io::Error::other(format!(
            "{error}; package changes and pending commit retained in {}; fix the cause and retry commit {}",
            pending.repository.display(),
            area.directory()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )));
    }
    let head = git::line(&pending.repository, &["rev-parse", "HEAD"])?;
    report.commit = Some(head.clone());
    if baseline::read(target)? != pending.after
        || super::super::recipe::files(&pending.repository, "HEAD", Path::new(&pending.package))?
            != pending.after
        || committed_paths(pending)? != changed
    {
        return Err(invalid(
            "commit was created, but hooks changed the planned content or scope; inspect the reported commit before retrying",
        ));
    }
    require_clean(&pending.repository)?;
    finish(pending, area, state, &head, report)
}

fn finish(
    pending: &Pending,
    area: &crate::workspace::Development,
    state: &Path,
    head: &str,
    report: &mut Report,
) -> io::Result<()> {
    report.commit = Some(head.to_owned());
    baseline::save(&area.directory().join("baseline.toml"), &pending.baseline)?;
    fs::remove_file(state)?;
    report.retained = false;
    Ok(())
}

pub(super) fn verify_work(
    pending: &Pending,
    area: &crate::workspace::Development,
) -> io::Result<()> {
    area.verify_binding()?;
    let files = baseline::read(area.package_directory())?;
    for name in changes(&pending.before, &pending.after) {
        if files.get(&name) != pending.after.get(&name) {
            return Err(invalid(format!(
                "{name}: WORK no longer matches the pending commit; repository changes retained"
            )));
        }
    }
    Ok(())
}

fn verify_head(pending: &Pending) -> io::Result<()> {
    if git::line(&pending.repository, &["symbolic-ref", "--quiet", "HEAD"])? != pending.branch
        || git::line(&pending.repository, &["rev-parse", "HEAD"])? != pending.parent
    {
        return Err(invalid(
            "repository branch or HEAD changed; package changes retained",
        ));
    }
    Ok(())
}

fn committed_paths(pending: &Pending) -> io::Result<Vec<String>> {
    let output = git::checked(
        &pending.repository,
        [
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-r",
            "-z",
            &pending.parent,
            "HEAD",
        ],
    )?;
    let prefix = format!("{}/", pending.package);
    let mut paths: Vec<String> = output
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|b| {
            std::str::from_utf8(b)
                .map_err(io::Error::other)?
                .strip_prefix(&prefix)
                .map(str::to_owned)
                .ok_or_else(|| invalid("commit includes files outside the package"))
        })
        .collect::<io::Result<_>>()?;
    paths.sort();
    Ok(paths)
}

fn check_status(pending: &Pending, changed: &[String]) -> io::Result<()> {
    let output = git::checked(
        &pending.repository,
        [
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?;
    for line in output.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let path = std::str::from_utf8(line.get(3..).ok_or_else(|| invalid("invalid git status"))?)
            .map_err(io::Error::other)?;
        if !changed
            .iter()
            .any(|name| path == format!("{}/{name}", pending.package))
        {
            return Err(invalid(format!(
                "unrelated repository change: {path}; pending commit retained"
            )));
        }
    }
    Ok(())
}

fn apply_files(
    pending: &Pending,
    source_directory: &Path,
    target: &Path,
    changed: &[String],
) -> io::Result<()> {
    for name in changed {
        let destination =
            super::super::directory(&pending.repository, Path::new(&pending.package), false)?
                .join(baseline::relative(name)?);
        if let Some(expected) = pending.after.get(name) {
            let from = source_directory.join(name);
            // Recheck every ancestor before writing into a target repository.
            super::super::directory(
                target,
                baseline::relative(name)?.parent().unwrap_or(Path::new("")),
                false,
            )?;
            fs::create_dir_all(destination.parent().expect("package parent"))?;
            let mut file =
                tempfile::NamedTempFile::new_in(destination.parent().expect("file parent"))?;
            let mut source = fs::File::open(&from)?;
            io::copy(&mut source, &mut file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.as_file()
                    .set_permissions(std::fs::Permissions::from_mode(if expected.executable {
                        0o755
                    } else {
                        0o644
                    }))?;
            }
            file.flush()?;
            file.as_file().sync_all()?;
            if crate::file_digest::read(file.path())
                .map_err(io::Error::other)?
                .sha256
                != expected.sha256
            {
                return Err(invalid(
                    "WORK changed while copying; partial publication retained",
                ));
            }
            file.persist(destination).map_err(|e| e.error)?;
        } else if destination.try_exists()? {
            fs::remove_file(destination)?;
        }
    }
    Ok(())
}
