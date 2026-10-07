// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Explicit cleanup of one receipt-bound project, never replaying stored commands.

use crate::environment::{invalid, process::Runner, regular_file};
use fs_err as fs;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn target(path: &Path) -> io::Result<PathBuf> {
    let literal: PathBuf = path.components().collect();
    if !fs::symlink_metadata(&literal)?.is_dir() {
        return Err(invalid("clean target must be a directory, not a symlink"));
    }
    let root = fs::canonicalize(&literal)?;
    let cwd = fs::canonicalize(std::env::current_dir()?)?;
    let home = fs::canonicalize(
        std::env::var_os("HOME")
            .ok_or_else(|| invalid("HOME is required for cleanup path protection"))?,
    )?;
    if root.parent().is_none() || cwd.starts_with(&root) || home.starts_with(&root) {
        return Err(invalid(
            "refusing to clean root, HOME, current directory, or any of their ancestors",
        ));
    }
    regular_file(&root.join("receipt.json"))?;
    match fs::symlink_metadata(root.join("host")) {
        Ok(metadata) if metadata.is_dir() => (),
        Ok(_) => {
            return Err(invalid(
                "clean host log directory must not be a symlink or non-directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    Ok(root)
}

fn required<'a>(value: &'a Value, field: &str) -> io::Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid(format!("build receipt requires nonempty {field}")))
}

fn docker(
    runner: &mut Runner<'_>,
    context: Option<&str>,
    start: Instant,
    timeout: Duration,
    stage: &str,
    args: &[&str],
) -> io::Result<String> {
    let command = crate::environment::compose::docker(context, args.iter().map(OsString::from));
    runner
        .capture(&command, stage, timeout.saturating_sub(start.elapsed()))
        .map_err(io::Error::other)
}

fn collect_resources(
    runner: &mut Runner<'_>,
    context: Option<&str>,
    start: Instant,
    timeout: Duration,
    attempt: &str,
    project: &str,
    report: &mut CleanReport,
) -> io::Result<()> {
    let filter = format!("label=com.docker.compose.project={project}");
    for kind in ["container", "network", "volume"] {
        let mut args = vec![kind, "ls", "--quiet"];
        if kind == "container" {
            args.push("--all");
        }
        if kind != "volume" {
            args.push("--no-trunc");
        }
        args.extend(["--filter", &filter]);
        let listed = docker(
            runner,
            context,
            start,
            timeout,
            &format!("{attempt}-list-{kind}"),
            &args,
        )?;
        let mut names = Vec::new();
        for name in listed
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let valid = if kind == "volume" {
                name.as_bytes()[0].is_ascii_alphanumeric()
                    && name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
                    })
            } else {
                name.bytes().all(|byte| byte.is_ascii_hexdigit())
            };
            if !valid {
                return Err(invalid(format!(
                    "Docker returned an invalid {kind} identity"
                )));
            }
            let inspected = docker(
                runner,
                context,
                start,
                timeout,
                &format!("{attempt}-inspect-{kind}"),
                &[kind, "inspect", name],
            )?;
            let inspected: Value = serde_json::from_str(&inspected).map_err(io::Error::other)?;
            let labels = if kind == "container" {
                &inspected[0]["Config"]["Labels"]
            } else {
                &inspected[0]["Labels"]
            };
            if labels["com.docker.compose.project"].as_str() != Some(project) {
                return Err(invalid(format!(
                    "{kind} {name} no longer has the exact project ownership label"
                )));
            }
            if kind == "volume"
                && labels["com.docker.compose.volume"]
                    .as_str()
                    .is_none_or(str::is_empty)
            {
                report.retained_volumes.push(name.to_owned());
            } else {
                names.push(name.to_owned());
            }
        }
        names.sort();
        names.dedup();
        report.scope.insert(kind, names);
    }
    Ok(())
}

fn collect_images(
    runner: &mut Runner<'_>,
    context: Option<&str>,
    start: Instant,
    timeout: Duration,
    project: &str,
    report: &mut CleanReport,
) -> io::Result<()> {
    let filter = format!("label=com.docker.compose.project={project}");
    let listed = docker(
        runner,
        context,
        start,
        timeout,
        "list-images",
        &["image", "ls", "--quiet", "--no-trunc", "--filter", &filter],
    )?;
    let mut images = std::collections::BTreeSet::new();
    for id in listed.lines().map(str::trim).filter(|id| !id.is_empty()) {
        let digest = id
            .strip_prefix("sha256:")
            .ok_or_else(|| invalid("invalid image digest"))?;
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid image digest"));
        }
        let info = docker(
            runner,
            context,
            start,
            timeout,
            "inspect-image",
            &["image", "inspect", id],
        )?;
        let info: Value = serde_json::from_str(&info).map_err(io::Error::other)?;
        if info[0]["Config"]["Labels"]["com.docker.compose.project"].as_str() != Some(project) {
            return Err(invalid("image ownership changed during cleanup"));
        }
        let tags = info[0]["RepoTags"].as_array();
        let shared_tag = tags.is_none_or(|tags| {
            tags.iter().any(|tag| {
                tag.as_str()
                    .is_none_or(|tag| !tag.starts_with(&format!("{project}-")))
            })
        });
        let users = docker(
            runner,
            context,
            start,
            timeout,
            "image-users",
            &[
                "container",
                "ls",
                "--all",
                "--quiet",
                "--no-trunc",
                "--filter",
                &format!("ancestor={id}"),
            ],
        )?;
        let shared_container = users.lines().any(|user| {
            !report.scope["container"]
                .iter()
                .any(|owned| owned == user.trim())
        });
        if shared_tag || shared_container {
            report.retained_images.push(id.to_owned());
        } else {
            images.insert(id.to_owned());
        }
    }
    report.scope.insert("image", images.into_iter().collect());
    Ok(())
}

fn receipt_identity<'a>(
    receipt: &'a Value,
    context: Option<&'a str>,
) -> io::Result<(&'a str, &'a str, Option<&'a str>)> {
    if receipt["format_version"] != 1 || receipt["backend"] != "compose" {
        return Err(invalid(
            "clean requires a format_version 1 Compose build receipt",
        ));
    }
    let details = &receipt["execution"]["details"];
    let project = required(details, "project")?;
    if !project.starts_with("ruyipack-")
        || project.len() <= 9
        || project.len() > 128
        || !project.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(invalid(
            "receipt project is not a valid ruyipack-* project name",
        ));
    }
    let daemon = required(details, "daemon_id")?;
    let stored_context = match receipt.get("context") {
        None | Some(Value::Null) => None,
        Some(Value::String(context)) => Some(context.as_str()),
        _ => return Err(invalid("receipt context must be a string or null")),
    };
    let context = context.or(stored_context);
    if context.is_some_and(|value| value.trim().is_empty()) {
        return Err(invalid("Docker context must not be empty"));
    }
    Ok((project, daemon, context))
}

fn perform(
    path: &Path,
    authorize: &mut dyn FnMut(&CleanReport) -> io::Result<()>,
    context: Option<&str>,
    timeout: Duration,
    report: &mut CleanReport,
    images: bool,
) -> io::Result<()> {
    let root = target(path)?;
    let receipt_path = root.join("receipt.json");
    // Serialize cleanup with shell sessions and other cleanups until local deletion ends.
    let file = fs::File::open(&receipt_path)?.into_file();
    let lock = crate::file_lock::FileLock::try_lock(file).map_err(|error| {
        io::Error::other(format!(
            "{}: another shell or cleanup session owns this build: {error}",
            receipt_path.display()
        ))
    })?;
    let mut original = Vec::new();
    lock.file().read_to_end(&mut original)?;
    let receipt: Value = serde_json::from_slice(&original).map_err(io::Error::other)?;
    if receipt["resources_retained"] == false {
        authorize(report)?;
        fs::remove_dir_all(&root)?;
        report.result_removed = true;
        return Ok(());
    }
    let (project, daemon, context) = receipt_identity(&receipt, context)?;
    report.project = Some(project.to_owned());
    report.daemon_id = Some(daemon.to_owned());
    report.context = context.map(str::to_owned);
    if !root.join("host").exists() {
        fs::create_dir(root.join("host"))?;
    }
    let attempt = crate::environment::compose::operation_id("clean");
    let mut runner = Runner {
        cancellable: false,
        output: &root,
        commands: Vec::new(),
    };
    let start = Instant::now();
    let outcome = (|| -> io::Result<()> {
        let info = docker(
            &mut runner,
            context,
            start,
            timeout,
            &format!("{attempt}-daemon"),
            &["info", "--format", "{{json .}}"],
        )?;
        let info: Value = serde_json::from_str(&info).map_err(io::Error::other)?;
        if info["ID"].as_str() != Some(daemon) {
            return Err(invalid(
                "Docker daemon identity differs from the build receipt; nothing was deleted",
            ));
        }
        collect_resources(
            &mut runner,
            context,
            start,
            timeout,
            &attempt,
            project,
            report,
        )?;
        if images {
            collect_images(&mut runner, context, start, timeout, project, report)?;
        }
        authorize(report)?;
        for (&kind, names) in &report.scope {
            for name in names {
                let mut args = vec![kind, "rm"];
                if kind == "container" {
                    args.push("--force");
                }
                args.push(name);
                docker(
                    &mut runner,
                    context,
                    start,
                    timeout,
                    &format!("{attempt}-remove-{kind}"),
                    &args,
                )?;
                report.removed.entry(kind).or_default().push(name.clone());
            }
        }
        Ok(())
    })();
    report.commands = runner.commands;
    if let Err(error) = &outcome {
        report.error = Some(error.to_string());
    }
    let saved = fs::write(
        root.join("host").join(format!("{attempt}.json")),
        serde_json::to_vec_pretty(report).map_err(io::Error::other)?,
    );
    outcome?;
    saved?;
    if target(path)? != root || fs::read(receipt_path)? != original {
        return Err(invalid(
            "build result identity changed during cleanup; local results were retained",
        ));
    }
    fs::remove_dir_all(&root)?;
    report.result_removed = true;
    Ok(())
}

pub(crate) fn execute(
    path: &Path,
    context: Option<&str>,
    timeout: Duration,
    authorize: &mut dyn FnMut(&CleanReport) -> io::Result<()>,
    images: bool,
) -> CleanReport {
    let mut report = new_report(path, context);
    match perform(path, authorize, context, timeout, &mut report, images) {
        Ok(()) => report.success = true,
        Err(error) => {
            report.cancelled = error.kind() == io::ErrorKind::Interrupted;
            report.error = Some(error.to_string());
        }
    }
    report
}

pub(crate) fn new_report(path: &Path, context: Option<&str>) -> CleanReport {
    CleanReport {
        format_version: 1,
        tool: crate::tool::identity(),
        operation: "clean",
        display_path: path.to_string_lossy().into_owned(),
        success: false,
        result_removed: false,
        cancelled: false,
        project: None,
        daemon_id: None,
        context: context.map(str::to_owned),
        scope: BTreeMap::new(),
        removed: ["container", "network", "volume"]
            .into_iter()
            .map(|kind| (kind, Vec::new()))
            .collect(),
        retained_volumes: Vec::new(),
        retained_images: Vec::new(),
        commands: Vec::new(),
        error: None,
    }
}

/// CLI evidence is typed; the retained build receipt protocol is migrated separately.
#[derive(Serialize)]
pub(crate) struct CleanReport {
    format_version: u32,
    tool: crate::tool::Identity,
    operation: &'static str,
    pub(crate) display_path: String,
    pub(crate) success: bool,
    result_removed: bool,
    pub(crate) cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) daemon_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    pub(crate) scope: BTreeMap<&'static str, Vec<String>>,
    pub(crate) removed: BTreeMap<&'static str, Vec<String>>,
    retained_volumes: Vec<String>,
    pub(crate) retained_images: Vec<String>,
    commands: Vec<crate::environment::CommandRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

/// Validate local evidence without deleting resources or contacting a daemon.
pub(crate) fn preflight(path: &Path) -> io::Result<()> {
    let root = target(path)?;
    let lock = crate::file_lock::FileLock::try_lock(
        fs::File::open(root.join("receipt.json"))?.into_file(),
    )?;
    let receipt: Value = serde_json::from_reader(lock.file()).map_err(io::Error::other)?;
    if receipt["format_version"] != 1 || receipt["backend"] != "compose" {
        return Err(invalid(
            "unsupported build receipt; clean or repair it before deletion",
        ));
    }
    if receipt["resources_retained"] != false {
        receipt_identity(&receipt, None)?;
    }
    Ok(())
}
