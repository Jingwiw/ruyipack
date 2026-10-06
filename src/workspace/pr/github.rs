// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! GitHub protocol boundary. JSON does not enter the package or plan model.

use super::{Options, Report, git, invalid, require_clean};
use serde::Deserialize;
use std::{
    io::{self, Write},
    path::Path,
    process::Command,
    time::Duration,
};

pub(super) fn repository(value: &str) -> io::Result<String> {
    let value = value
        .strip_prefix("git@github.com:")
        .or_else(|| value.strip_prefix("https://github.com/"))
        .unwrap_or(value)
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        return Err(invalid(
            "PR repository must be a GitHub OWNER/REPO or GitHub HTTPS/SSH URL",
        ));
    }
    Ok(value.to_owned())
}

fn gh(repo: &Path, args: &[&str], timeout: u64) -> io::Result<Vec<u8>> {
    let mut command = Command::new("gh");
    command
        .current_dir(repo)
        .args(["api", "--hostname", "github.com"])
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env_remove("GH_REPO");
    let output =
        crate::host_process::capture(&mut command, Duration::from_secs(timeout), 2 * 1024 * 1024)?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "GitHub request failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

#[derive(Deserialize)]
struct PullRequest {
    number: u64,
    state: String,
    merged_at: Option<String>,
    html_url: String,
    title: String,
    body: Option<String>,
    head: Head,
}
#[derive(Deserialize)]
struct Head {
    sha: String,
}

pub(super) fn publish(
    repo: &Path,
    origin: &str,
    options: &Options,
    report: &mut Report,
) -> io::Result<()> {
    report.stage = "remote-preflight";
    let endpoint = format!("repos/{}/pulls", report.target);
    let base_endpoint = format!("repos/{}/commits", report.target);
    let base = gh(
        repo,
        &[
            &base_endpoint,
            "--method",
            "GET",
            "-f",
            &format!("sha={}", report.base),
            "-f",
            "per_page=1",
            "--jq",
            ".[0].sha",
        ],
        options.timeout,
    )?;
    if String::from_utf8_lossy(&base).trim() != report.base_revision {
        return Err(invalid(
            "target base changed; fetch it and review the PR range again",
        ));
    }
    let owner = origin.split_once('/').expect("validated repository").0;
    let head = format!("{owner}:{}", report.branch);
    let existing = matching(
        repo,
        &endpoint,
        &head,
        &report.base,
        "open",
        options.timeout,
    )?;
    // Recheck after remote requests. Push the observed object, not a moving HEAD.
    require_clean(repo)?;
    if git::line(repo, &["rev-parse", "HEAD"])? != report.head
        || git::line(repo, &["symbolic-ref", "--short", "HEAD"])? != report.branch
    {
        return Err(invalid("branch changed during PR preparation"));
    }
    let push_urls = git::line(repo, &["remote", "get-url", "--push", "--all", "origin"])?;
    if push_urls.lines().count() != 1 || repository(&push_urls)? != origin {
        return Err(invalid(
            "origin push URL differs from the reviewed repository",
        ));
    }
    report.stage = "push";
    report.push_attempted = true;
    git::checked_with_budget(
        repo,
        [
            "push",
            "--",
            "origin",
            &format!("{}:refs/heads/{}", report.head, report.branch),
        ],
        Duration::from_secs(options.timeout),
    )?;
    report.pushed = true;
    report.stage = "pull-request";
    let payload = if existing.is_empty() {
        serde_json::json!({"title": report.title, "body": report.body, "head": head, "base": report.base, "draft": true})
    } else {
        serde_json::json!({"title": report.title, "body": report.body})
    };
    let mut file = tempfile::NamedTempFile::new()?;
    serde_json::to_writer(file.as_file_mut(), &payload).map_err(io::Error::other)?;
    file.flush()?;
    let (method, path) = existing.first().map_or_else(
        || ("POST", endpoint.clone()),
        |pr| ("PATCH", format!("{endpoint}/{}", pr.number)),
    );
    let result: PullRequest = serde_json::from_slice(&gh(
        repo,
        &[
            &path,
            "--method",
            method,
            "--input",
            git::text(file.path())?,
        ],
        options.timeout,
    )?)
    .map_err(io::Error::other)?;
    report.url = Some(result.html_url);
    if result.head.sha != report.head
        || result.title != report.title
        || result.body.as_deref() != Some(&report.body)
    {
        return Err(invalid(
            "PR response differs from prepared content; inspect the returned URL before retrying",
        ));
    }
    report.published = true;
    report.stage = "published";
    Ok(())
}

fn matching(
    repo: &Path,
    endpoint: &str,
    head: &str,
    base: &str,
    state: &str,
    timeout: u64,
) -> io::Result<Vec<PullRequest>> {
    let pulls: Vec<PullRequest> = serde_json::from_slice(&gh(
        repo,
        &[
            endpoint,
            "--method",
            "GET",
            "-f",
            &format!("state={state}"),
            "-f",
            &format!("head={head}"),
            "-f",
            &format!("base={base}"),
        ],
        timeout,
    )?)
    .map_err(io::Error::other)?;
    if state == "open" && pulls.len() > 1 {
        return Err(invalid("multiple open PRs match this branch and base"));
    }
    Ok(pulls)
}

#[derive(Deserialize)]
struct Runs {
    workflow_runs: Vec<Run>,
}
#[derive(Deserialize)]
struct Run {
    id: u64,
    head_sha: String,
    head_branch: Option<String>,
    status: String,
    event: String,
    pull_requests: Vec<Number>,
}
#[derive(Deserialize)]
struct Number {
    number: u64,
}

pub(super) fn close(
    repo: &Path,
    origin: &str,
    options: &Options,
    report: &mut Report,
) -> io::Result<()> {
    report.stage = "close-preflight";
    let endpoint = format!("repos/{}/pulls", report.target);
    let owner = origin.split_once('/').expect("validated repository").0;
    let mut pulls = matching(
        repo,
        &endpoint,
        &format!("{owner}:{}", report.branch),
        &report.base,
        "open",
        options.timeout,
    )?;
    if pulls.is_empty() {
        pulls = matching(
            repo,
            &endpoint,
            &format!("{owner}:{}", report.branch),
            &report.base,
            "closed",
            options.timeout,
        )?;
        pulls.retain(|pr| pr.head.sha == report.head && pr.merged_at.is_none());
    }
    let pr = pulls
        .first()
        .ok_or_else(|| invalid("no unmerged PR matches this branch, base and head"))?;
    report.url = Some(pr.html_url.clone());
    if pr.head.sha != report.head {
        return Err(invalid(
            "PR head differs from local HEAD; review before closing",
        ));
    }
    // List before cancelling: cancellation can change pagination filters.
    let mut runs = Vec::new();
    for page in 1.. {
        let result: Runs = serde_json::from_slice(&gh(
            repo,
            &[
                &format!("repos/{}/actions/runs", report.target),
                "--method",
                "GET",
                "-f",
                &format!("branch={}", report.branch),
                "-f",
                "per_page=100",
                "-f",
                &format!("page={page}"),
            ],
            options.timeout,
        )?)
        .map_err(io::Error::other)?;
        let done = result.workflow_runs.len() < 100;
        runs.extend(result.workflow_runs.into_iter().filter(|run| {
            run.status != "completed"
                && (run
                    .pull_requests
                    .iter()
                    .any(|pull| pull.number == pr.number)
                    || (origin == report.target
                        && report.commits.contains(&run.head_sha)
                        && run.event == "push"
                        && run.head_branch.as_deref() == Some(&report.branch)))
        }));
        if done {
            break;
        }
    }
    report.stage = "close";
    let result: PullRequest = serde_json::from_slice(&gh(
        repo,
        &[
            &format!("{endpoint}/{}", pr.number),
            "--method",
            "PATCH",
            "-f",
            "state=closed",
        ],
        options.timeout,
    )?)
    .map_err(io::Error::other)?;
    report.closed = result.state == "closed";
    if !report.closed || result.head.sha != report.head {
        return Err(invalid("PR close response differs from the reviewed PR"));
    }
    report.stage = "cancel-actions";
    for run in runs {
        let path = format!("repos/{}/actions/runs/{}", report.target, run.id);
        // Recheck: a run may have finished while the PR was closing.
        let current: Run = serde_json::from_slice(&gh(repo, &[&path], options.timeout)?)
            .map_err(io::Error::other)?;
        if current.status == "completed" {
            continue;
        }
        gh(
            repo,
            &[&format!("{path}/cancel"), "--method", "POST"],
            options.timeout,
        )?;
        report.cancel_requested.push(run.id);
        // A cancellation request is not proof of termination.
        let deadline = std::time::Instant::now() + Duration::from_secs(options.timeout);
        loop {
            let current: Run = serde_json::from_slice(&gh(repo, &[&path], options.timeout)?)
                .map_err(io::Error::other)?;
            if current.status == "completed" {
                report.finished_runs.push(run.id);
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(invalid(format!(
                    "run {}: cancellation requested but completion not confirmed",
                    run.id
                )));
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    report.stage = "closed";
    Ok(())
}
