// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Bounded Git commands; callers select paths and own publication policy.

use super::invalid;
use fs_err as fs;
use std::{
    ffi::OsStr,
    io,
    path::Path,
    process::{Command, Output},
    time::Duration,
};

pub(super) fn line(repo: &Path, args: &[&str]) -> io::Result<String> {
    let output = String::from_utf8(checked(repo, args)?).map_err(io::Error::other)?;
    Ok(output.strip_suffix('\n').unwrap_or(&output).to_owned())
}

pub(super) fn text(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| {
        invalid(format!(
            "Git selection path {} must be UTF-8",
            path.display()
        ))
    })
}

pub(super) fn checked(
    repo: &Path,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> io::Result<Vec<u8>> {
    checked_with_budget(repo, args, Duration::from_secs(30))
}

pub(super) fn checked_with_budget(
    repo: &Path,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    budget: Duration,
) -> io::Result<Vec<u8>> {
    let args: Vec<_> = args.into_iter().collect();
    let output = git_with_budget(repo, &args, budget)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(command_error(
            repo,
            &args[0].as_ref().to_string_lossy(),
            &output,
        ))
    }
}

pub(super) fn clone_repository(destination: &Path, url: &OsStr) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(invalid(format!(
                "{} already exists; inspect it or choose an unused recipes path before retrying",
                destination.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // Relative local URLs use the caller's directory, not the destination's.
    // Git creates missing destination parents itself.
    let current = std::env::current_dir()?;
    let output = git_with_budget(
        &current,
        [
            OsStr::new("clone"),
            OsStr::new("--"),
            url,
            destination.as_os_str(),
        ],
        Duration::from_secs(300),
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(&current, "clone", &output))
    }
}

fn git_with_budget(
    repo: &Path,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    budget: Duration,
) -> io::Result<Output> {
    let mut command = Command::new("git");
    command
        .args(["--literal-pathspecs", "-c", "core.fsmonitor=false", "-C"])
        .arg(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1");
    for key in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(key);
    }
    crate::host_process::capture(&mut command, budget, 8 * 1024 * 1024).map_err(|error| {
        io::Error::new(error.kind(), format!("Git in {}: {error}", repo.display()))
    })
}

fn command_error(repo: &Path, command: &str, output: &Output) -> io::Error {
    io::Error::other(format!(
        "git {command} in {} failed ({}): {}",
        repo.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim_end()
    ))
}
