// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! One read-only OBS observation. Submission success is not build success.

use super::{api, config, delivery};
use crate::{output_cli::ReportFormat, workspace::Workspace};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum State {
    Passed,
    Waiting,
    Failed,
    Stale,
    Unavailable,
}

#[derive(Serialize)]
struct Target {
    repository: String,
    architecture: String,
    code: String,
    state: State,
    revision: Option<String>,
    source_md5: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Observation {
    pub(crate) work: String,
    project: Option<String>,
    package: Option<String>,
    revision: Option<String>,
    pub(crate) state: State,
    source_md5: Option<String>,
    service_md5: Option<String>,
    targets: Vec<Target>,
    pub(crate) error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Report {
    format_version: u32,
    operation: &'static str,
    scope: &'static str,
    observed_at: i64,
    read_requests: usize,
    success: bool,
    pub(crate) tasks: Vec<Observation>,
}

pub(crate) fn run(
    workspace: &Workspace,
    works: &[String],
    format: ReportFormat,
) -> Result<bool, String> {
    let report = collect(workspace, works)?;
    if matches!(format, ReportFormat::Human) {
        for row in &report.tasks {
            crate::output_cli::stderr()
                .message(
                    if row.state == State::Passed {
                        crate::output_cli::HumanLevel::Info
                    } else {
                        crate::output_cli::HumanLevel::Warn
                    },
                    Some(std::path::Path::new(&row.work)),
                    format_args!(
                        "remote-build={:?}{}",
                        row.state,
                        row.error
                            .as_ref()
                            .map_or(String::new(), |e| format!("; {e}"))
                    ),
                )
                .map_err(|e| e.to_string())?;
        }
    }
    if matches!(format, ReportFormat::Toml) {
        crate::report::write(&mut io::stdout().lock(), &report).map_err(|e| e.to_string())?;
    }
    Ok(report.success)
}

pub(crate) fn collect(workspace: &Workspace, works: &[String]) -> Result<Report, String> {
    crate::plan::validate_works(works.iter().map(String::as_str))?;
    let (global, auth) = config::load_existing(&workspace.configuration())?;
    let client = api::Client::new(&global.api, &auth.user, &auth.password)?;
    // Include failures in the cache: an unavailable project is queried once per round.
    let mut projects: BTreeMap<String, Result<(String, String), String>> = BTreeMap::new();
    let mut tasks = Vec::new();
    for work in works {
        let mut row = Observation {
            work: work.clone(),
            project: None,
            package: None,
            revision: None,
            state: State::Unavailable,
            source_md5: None,
            service_md5: None,
            targets: vec![],
            error: None,
        };
        if let Err(error) = observe(
            workspace,
            &client,
            &auth.user,
            work,
            &mut projects,
            &mut row,
        ) {
            row.error = Some(error);
        }
        tasks.push(row);
    }
    let report = Report {
        format_version: 1,
        operation: "remote-build-status",
        scope: "obs-source-and-target-matrix",
        observed_at: time::OffsetDateTime::now_utc().unix_timestamp(),
        read_requests: client.read_requests(),
        success: tasks.iter().all(|row| row.state == State::Passed),
        tasks,
    };
    Ok(report)
}

fn observe(
    workspace: &Workspace,
    client: &api::Client,
    user: &str,
    work: &str,
    projects: &mut BTreeMap<String, Result<(String, String), String>>,
    row: &mut Observation,
) -> Result<(), String> {
    let area = workspace
        .existing_development(work)
        .map_err(|e| e.to_string())?;
    let settings: config::Settings = config::read(&area.directory().join("remote.toml"))?;
    let receipt: delivery::Receipt = config::read(&area.directory().join("remote-receipt.toml"))?;
    api::owned_project(&receipt.project, user)?;
    row.project = Some(receipt.project.clone());
    row.package = Some(receipt.package.clone());
    row.revision = Some(receipt.revision.clone());
    if receipt.api != client.origin()
        || receipt.package != area.package()
        || settings.project.as_deref() != Some(receipt.project.as_str())
    {
        return Err("OBS settings no longer match the submitted receipt".into());
    }
    let (meta, results) = projects
        .entry(receipt.project.clone())
        .or_insert_with(|| {
            let meta = client
                .get(&["source", &receipt.project, "_meta"])?
                .ok_or("OBS project not found")?;
            let results = client
                .get_query(
                    &["build", &receipt.project, "_result"],
                    &[("view", "status"), ("view", "info")],
                )?
                .ok_or("OBS results not available")?;
            Ok((meta, results))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let source = client
        .get(&["source", &receipt.project, &receipt.package])?
        .ok_or("OBS package not found")?;
    let document = api::parse(&source)?;
    let root = document.root_element();
    row.source_md5 = root.attribute("srcmd5").map(str::to_owned);
    row.service_md5 = root
        .children()
        .find(|n| n.has_tag_name("serviceinfo"))
        .and_then(|n| n.attribute("xsrcmd5"))
        .map(str::to_owned);
    if root.attribute("rev") != Some(receipt.revision.as_str())
        || delivery::directory(&source)? != receipt.files
    {
        row.state = State::Stale;
        return Err("OBS source differs from the submitted receipt".into());
    }
    let source_md5 = if let Some(service) = root.children().find(|n| n.has_tag_name("serviceinfo"))
    {
        match service.attribute("code") {
            Some("running") => {
                row.state = State::Waiting;
                return Ok(());
            }
            Some("failed") => {
                row.state = State::Failed;
                return Err("OBS source service failed".into());
            }
            Some("succeeded") => service.attribute("xsrcmd5"),
            _ => return Err("OBS source service identity is unavailable".into()),
        }
    } else {
        root.attribute("srcmd5")
    }
    .filter(|s| !s.is_empty())
    .ok_or("OBS source has no build content identity")?;
    row.targets = targets(meta, results, &receipt, &settings, source_md5)?;
    row.state = aggregate(&row.targets);
    Ok(())
}

fn targets(
    meta: &str,
    results: &str,
    receipt: &delivery::Receipt,
    settings: &config::Settings,
    source_md5: &str,
) -> Result<Vec<Target>, String> {
    let meta = api::parse(meta)?;
    let results = api::parse(results)?;
    if meta.root_element().attribute("name") != Some(receipt.project.as_str())
        || results.root_element().tag_name().name() != "resultlist"
    {
        return Err("OBS response identifies an unexpected project or result type".into());
    }
    let repositories = settings
        .repositories
        .as_ref()
        .filter(|r| !r.is_empty())
        .ok_or("no configured repositories")?;
    let mut expected = BTreeSet::new();
    for repository in repositories {
        let entries: Vec<_> = meta
            .root_element()
            .children()
            .filter(|n| n.has_tag_name("repository") && n.attribute("name") == Some(repository))
            .collect();
        let [entry] = entries.as_slice() else {
            return Err(format!("repository missing or duplicated: {repository}"));
        };
        let mut count = 0;
        for arch in entry.children().filter(|n| n.has_tag_name("arch")) {
            let arch = arch
                .text()
                .filter(|s| !s.is_empty())
                .ok_or("empty architecture")?;
            if !expected.insert((repository.clone(), arch.to_owned())) {
                return Err("duplicate configured target".into());
            }
            count += 1;
        }
        if count == 0 {
            return Err(format!("repository has no architectures: {repository}"));
        }
    }
    let mut targets = Vec::new();
    for (repository, architecture) in expected {
        let matching: Vec<_> = results
            .root_element()
            .children()
            .filter(|n| {
                n.has_tag_name("result")
                    && n.attribute("project") == Some(receipt.project.as_str())
                    && n.attribute("repository") == Some(repository.as_str())
                    && n.attribute("arch") == Some(architecture.as_str())
            })
            .collect();
        let mut target = Target {
            repository,
            architecture,
            code: "missing".into(),
            state: State::Waiting,
            revision: None,
            source_md5: None,
        };
        if matching.len() > 1 {
            return Err("duplicate OBS target result".into());
        }
        if let Some(result) = matching.first() {
            let statuses: Vec<_> = result
                .children()
                .filter(|n| {
                    n.has_tag_name("status")
                        && n.attribute("package") == Some(receipt.package.as_str())
                })
                .collect();
            if statuses.len() > 1 {
                return Err("duplicate OBS package status".into());
            }
            if let Some(status) = statuses.first() {
                target.code = status.attribute("code").unwrap_or("unknown").into();
                let infos: Vec<_> = result
                    .children()
                    .filter(|n| {
                        n.has_tag_name("info")
                            && n.attribute("package") == Some(receipt.package.as_str())
                    })
                    .collect();
                if infos.len() > 1 {
                    return Err("duplicate OBS build identity".into());
                }
                if let Some(info) = infos.first() {
                    target.revision = child_text(*info, "rev");
                    target.source_md5 = child_text(*info, "srcmd5");
                }
                let dirty =
                    result.attribute("dirty").is_some() || status.attribute("dirty").is_some();
                target.state = classify(
                    &target.code,
                    dirty,
                    target.revision.as_deref() == Some(&receipt.revision)
                        && target.source_md5.as_deref() == Some(source_md5),
                );
            }
        }
        targets.push(target);
    }
    Ok(targets)
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|n| n.has_tag_name(name))
        .and_then(|n| n.text())
        .map(str::to_owned)
}

fn classify(code: &str, dirty: bool, identity_matches: bool) -> State {
    if dirty {
        return State::Waiting;
    }
    match code {
        "succeeded" if identity_matches => State::Passed,
        "succeeded" => State::Unavailable,
        "scheduled" | "building" | "blocked" | "signing" | "dispatching" => State::Waiting,
        "failed" | "unresolvable" | "broken" | "disabled" | "excluded" => State::Failed,
        _ => State::Unavailable,
    }
}

fn aggregate(targets: &[Target]) -> State {
    if targets.is_empty() {
        return State::Unavailable;
    }
    for state in [State::Failed, State::Unavailable, State::Waiting] {
        if targets.iter().any(|t| t.state == state) {
            return state;
        }
    }
    State::Passed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_inventory_rejects_partial_and_duplicate_records() {
        for invalid in [
            "<directory><entry name='a'/></directory>",
            "<directory><entry md5='x'/></directory>",
            "<directory><entry name='a' md5='x'/><entry name='a' md5='y'/></directory>",
        ] {
            assert!(delivery::directory(invalid).is_err());
        }
        assert_eq!(
            delivery::directory(
                "<directory><entry name='a' md5='x'/><entry name='_service:generated'/></directory>"
            )
            .unwrap(),
            BTreeMap::from([("a".into(), "x".into())])
        );
    }

    #[test]
    fn success_requires_current_clean_identity_and_every_configured_target() {
        let receipt = delivery::Receipt {
            api: "https://obs.example".into(),
            project: "home:a:p".into(),
            package: "pkg".into(),
            revision: "2".into(),
            files: BTreeMap::new(),
            sha256: BTreeMap::new(),
        };
        let settings = config::Settings {
            repositories: Some(vec!["riscv64".into(), "rva20".into()]),
            ..Default::default()
        };
        let meta = "<project name='home:a:p'><repository name='riscv64'><arch>riscv64</arch></repository><repository name='rva20'><arch>riscv64</arch></repository></project>";
        let one = "<result project='home:a:p' repository='riscv64' arch='riscv64'><status package='pkg' code='succeeded'/><info package='pkg'><rev>2</rev><srcmd5>abc</srcmd5></info></result>";
        let complete = format!(
            "<resultlist>{one}{}</resultlist>",
            one.replace("repository='riscv64'", "repository='rva20'")
        );
        let check =
            |input: &str| aggregate(&targets(meta, input, &receipt, &settings, "abc").unwrap());
        assert_eq!(check(&complete), State::Passed);
        assert_eq!(
            check(&format!("<resultlist>{one}</resultlist>")),
            State::Waiting
        );
        assert_eq!(
            check(&complete.replace("<rev>2", "<rev>1")),
            State::Unavailable
        );
        assert_eq!(
            check(&complete.replace("<srcmd5>abc", "<srcmd5>old")),
            State::Unavailable
        );
        assert_eq!(
            check(&complete.replace("<status ", "<status dirty='true' ")),
            State::Waiting
        );
        assert_eq!(
            check(&complete.replace("code='succeeded'", "code='failed'")),
            State::Failed
        );
        assert_eq!(
            check(&complete.replace("code='succeeded'", "code='new-state'")),
            State::Unavailable
        );
        assert!(
            targets(
                meta,
                &format!("<resultlist>{one}{one}</resultlist>"),
                &receipt,
                &settings,
                "abc"
            )
            .is_err()
        );
    }
}
