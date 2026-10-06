// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Submit immutable local package inputs to an owned OBS home project.
mod api;
mod config;
mod delivery;
use crate::output_cli::ReportFormat;
use crate::plan::Task;
use clap::Args;
use config::Settings;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::PathBuf,
};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("remote-input").args(["work","plan"]).required(true)))]
pub(crate) struct Options {
    /// Package development area.
    work: Option<String>,
    /// Read tasks and defaults from a plan; repeat to combine plans.
    #[arg(long, value_name = "PATH")]
    plan: Vec<PathBuf>,
    /// Parent repositories to enable; overrides plan and saved WORK settings.
    #[arg(long, value_delimiter = ',', value_name = "REPOSITORY")]
    repositories: Vec<String>,
    /// Import settings from another WORK's remote.toml.
    #[arg(long, requires = "work", value_name = "PATH")]
    from_config: Option<PathBuf>,
    /// Submit changed materials and refresh remote services; otherwise show retained results.
    #[arg(long)]
    fresh: bool,
    #[arg(long,value_enum,default_value_t=ReportFormat::Human)]
    format: ReportFormat,
}
#[derive(Serialize)]
struct ResultRow {
    work: String,
    project: Option<String>,
    package: Option<String>,
    action: &'static str,
    revision: Option<String>,
    uploaded: Vec<String>,
    removed: Vec<String>,
    builds: Vec<Build>,
    error: Option<String>,
}
#[derive(Serialize)]
struct Build {
    repository: String,
    architecture: String,
    status: String,
    details: Option<String>,
}
#[derive(Serialize)]
struct Report {
    format_version: u32,
    operation: &'static str,
    scope: &'static str,
    success: bool,
    tasks: Vec<ResultRow>,
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let workspace = crate::workspace::discover().map_err(|e| e.to_string())?;
    let interactive = matches!(options.format, ReportFormat::Human)
        && io::stdin().is_terminal()
        && io::stderr().is_terminal();
    let (global, auth) = config::load(&workspace.configuration(), interactive)?;
    let client = api::Client::new(&global.api, &auth.user, &auth.password)?;
    // Authentication is checked before any remote mutation.
    client
        .get(&["person", &auth.user])?
        .ok_or("OBS account not found")?;
    let mut tasks = Vec::new();
    if options.plan.is_empty() {
        let settings: Settings = options
            .from_config
            .as_ref()
            .map(|p| config::read(p))
            .transpose()?
            .unwrap_or_default();
        tasks.push(Task {
            work: options.work.clone().expect("WORK or plan"),
            settings: settings.inherit(&global.defaults),
        });
    } else {
        for path in &options.plan {
            let plan: config::Plan = config::read(path)?;
            let defaults = plan.defaults.inherit(&global.defaults);
            tasks.extend(plan.packages.into_iter().map(|task| Task {
                work: task.work,
                settings: task.settings.inherit(&defaults),
            }));
        }
    }
    if tasks.is_empty() {
        return Err("plan contains no packages".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut projects = BTreeMap::new();
    let mut prepared = Vec::new();
    for task in tasks {
        if !seen.insert(task.work.clone()) {
            return Err(format!("duplicate WORK in plan: {}", task.work));
        }
        let mut settings = task.settings;
        let mut area = workspace
            .development(&task.work, None, false)
            .map_err(|e| e.to_string())?;
        area.create().map_err(|e| e.to_string())?;
        let path = area.directory().join("remote.toml");
        if path.exists() && options.plan.is_empty() && options.from_config.is_none() {
            settings = config::read(&path)?;
        } else if options.plan.is_empty() && options.from_config.is_none() {
            if !interactive {
                return Err(
                    "first remote-build needs an interactive terminal, --from-config, or --plan"
                        .into(),
                );
            }
            configure(&client, &task.work, &auth.user, &mut settings)?;
        }
        if !options.repositories.is_empty() {
            settings.repositories = Some(
                options
                    .repositories
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            );
        }
        settings
            .project
            .get_or_insert_with(|| format!("home:{}:ruyipack-{}", auth.user, task.work));
        normalize_parent(&global.api, &mut settings)?;
        api::owned_project(
            settings.project.as_deref().ok_or("missing project")?,
            &auth.user,
        )?;
        if settings.repositories.as_ref().is_none_or(Vec::is_empty) {
            return Err("select at least one parent repository".into());
        }
        let key = settings.project.clone().expect("project");
        let signature = (
            settings.parent.clone(),
            settings.repositories.clone(),
            settings.publish,
        );
        if projects
            .insert(key, signature.clone())
            .is_some_and(|old| old != signature)
        {
            return Err("tasks sharing an OBS project must use the same project settings".into());
        }
        config::save(&path, &settings)?;
        prepared.push((task.work, settings));
    }
    // Project configuration is independent of package uploads and --fresh.
    let mut configured = std::collections::BTreeSet::new();
    for (_, settings) in &prepared {
        if configured.insert(settings.project.as_deref().expect("project")) {
            delivery::project(&client, &auth.user, settings)?;
        }
    }
    let mut rows = Vec::new();
    for (work, settings) in prepared {
        let mut row = ResultRow {
            work: work.clone(),
            project: settings.project.clone(),
            package: None,
            action: "not-submitted",
            revision: None,
            uploaded: vec![],
            removed: vec![],
            builds: vec![],
            error: None,
        };
        if let Err(error) = execute(
            &workspace,
            &client,
            &work,
            &settings,
            options.fresh,
            &mut row,
        ) {
            row.error = Some(error);
        }
        if let Ok(area) = workspace.existing_development(&work) {
            config::save(&area.directory().join("remote-result.toml"), &row)?;
        }
        if matches!(options.format, ReportFormat::Human) {
            let level = if row.error.is_some() {
                crate::output_cli::HumanLevel::Error
            } else {
                crate::output_cli::HumanLevel::Info
            };
            crate::output_cli::stderr()
                .message(
                    level,
                    Some(std::path::Path::new(&work)),
                    format_args!(
                        "remote-build: {}; project={}; uploaded={}; removed={}{}",
                        row.action,
                        row.project.as_deref().unwrap_or(""),
                        row.uploaded.len(),
                        row.removed.len(),
                        row.error
                            .as_ref()
                            .map_or(String::new(), |e| format!("; {e}"))
                    ),
                )
                .map_err(|e| e.to_string())?;
            for b in &row.builds {
                println!(
                    "{work}: {}/{} {}{}",
                    b.repository,
                    b.architecture,
                    b.status,
                    b.details
                        .as_ref()
                        .map_or(String::new(), |d| format!(": {d}"))
                );
            }
        }
        rows.push(row);
    }
    let report = Report {
        format_version: 1,
        operation: "remote-build",
        scope: "obs-submission",
        success: rows.iter().all(|r| r.error.is_none()),
        tasks: rows,
    };
    if matches!(options.format, ReportFormat::Toml) {
        crate::report::write(&mut io::stdout().lock(), &report).map_err(|e| e.to_string())?;
    }
    Ok(report.success)
}
fn normalize_parent(api: &str, settings: &mut Settings) -> Result<(), String> {
    let parent = settings.parent.get_or_insert_with(|| "openruyi".into());
    if parent.contains("://") {
        let url = url::Url::parse(parent).map_err(|e| e.to_string())?;
        let api = url::Url::parse(api).map_err(|e| e.to_string())?;
        if url.origin() != api.origin() {
            return Err("parent project URL must use the configured OBS origin".into());
        }
        *parent = url
            .path()
            .strip_prefix("/project/show/")
            .filter(|s| !s.is_empty() && !s.contains('/'))
            .ok_or("expected OBS /project/show/PROJECT URL")?
            .into();
    }
    for part in parent.split(':') {
        api::identifier(part)?;
    }
    Ok(())
}
fn configure(
    client: &api::Client,
    work: &str,
    user: &str,
    settings: &mut Settings,
) -> Result<(), String> {
    settings.project = Some(
        crate::prompt::value(
            "OBS project",
            settings
                .project
                .as_deref()
                .unwrap_or(&format!("home:{user}:ruyipack-{work}")),
        )
        .map_err(|e| e.to_string())?,
    );
    settings.parent = Some(
        crate::prompt::value(
            "Parent project",
            settings.parent.as_deref().unwrap_or("openruyi"),
        )
        .map_err(|e| e.to_string())?,
    );
    normalize_parent(&client.origin(), settings)?;
    let parent = settings.parent.as_deref().ok_or("missing parent")?;
    let metadata = client
        .get(&["source", parent, "_meta"])?
        .ok_or("parent project not found")?;
    let doc = api::parse(&metadata)?;
    let choices: Vec<_> = doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("repository"))
        .filter_map(|n| n.attribute("name"))
        .collect();
    let default: Vec<_> = choices
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            settings
                .repositories
                .as_ref()
                .is_some_and(|r| r.iter().any(|x| x == **n))
        })
        .map(|(i, _)| i)
        .collect();
    settings.repositories = Some(
        inquire::MultiSelect::new(
            "Parent repositories (architectures follow repository metadata):",
            choices,
        )
        .with_default(&default)
        .prompt()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(str::to_owned)
        .collect(),
    );
    settings.publish = Some(
        inquire::Confirm::new("Publish RPM repository?")
            .with_default(settings.publish.unwrap_or(false))
            .prompt()
            .map_err(|e| e.to_string())?,
    );
    Ok(())
}
fn execute(
    workspace: &crate::workspace::Workspace,
    client: &api::Client,
    work: &str,
    settings: &Settings,
    fresh: bool,
    row: &mut ResultRow,
) -> Result<(), String> {
    let mut area = workspace
        .existing_development(work)
        .map_err(|e| e.to_string())?;
    let project = settings.project.as_deref().ok_or("missing project")?;
    let package = area.package().to_owned();
    row.package = Some(package.clone());
    let receipt_path = area.directory().join("remote-receipt.toml");
    let previous: Option<delivery::Receipt> = if receipt_path.exists() {
        Some(config::read(&receipt_path)?)
    } else {
        None
    };
    if let Some(previous) = &previous {
        if previous.api != client.origin()
            || previous.project != project
            || previous.package != package
        {
            return Err("remote settings differ from retained receipt; use a new WORK".into());
        }
        row.revision = Some(previous.revision.clone());
        row.action = "retained";
    }
    if previous.is_none() || fresh {
        workspace
            .materialize(&mut area)
            .map_err(|e| e.to_string())?;
        let service = if let Some(path) = &settings.service {
            crate::utf8_file::read(&workspace.configuration().join(path))
                .map_err(|e| e.to_string())?
        } else if area.directory().join("_service").exists() {
            crate::utf8_file::read(&area.directory().join("_service")).map_err(|e| e.to_string())?
        } else if area.package_directory().join("_service").exists() {
            crate::utf8_file::read(&area.package_directory().join("_service"))
                .map_err(|e| e.to_string())?
        } else {
            let path = workspace.configuration().join("obs-service.xml");
            if path.exists() {
                crate::utf8_file::read(&path).map_err(|e| e.to_string())?
            } else {
                config::SERVICE.into()
            }
        };
        let constraints_path = settings.constraints.as_ref().map_or_else(
            || area.directory().join("_constraints"),
            |path| workspace.configuration().join(path),
        );
        let constraints = if constraints_path.try_exists().map_err(|e| e.to_string())? {
            Some(crate::utf8_file::read(&constraints_path).map_err(|e| e.to_string())?)
        } else if settings.constraints.is_some() {
            return Err(format!(
                "constraints file not found: {}",
                constraints_path.display()
            ));
        } else {
            None
        };
        let input = delivery::stage(&area, &service, constraints.as_deref())?;
        let (receipt, uploaded, removed) =
            delivery::submit(client, project, &package, &input, previous.as_ref(), fresh)?;
        row.revision = Some(receipt.revision.clone());
        row.uploaded = uploaded;
        row.removed = removed;
        row.action = if row.uploaded.is_empty() && row.removed.is_empty() {
            "unchanged"
        } else {
            "submitted"
        };
        config::save(&receipt_path, &receipt)?;
    }
    let results = client
        .get(&["build", project, "_result"])?
        .ok_or("OBS results not available")?;
    let doc = api::parse(&results)?;
    for result in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("result"))
    {
        for status in result.children().filter(|n| {
            n.has_tag_name("status") && n.attribute("package") == Some(package.as_str())
        }) {
            row.builds.push(Build {
                repository: result.attribute("repository").unwrap_or("").into(),
                architecture: result.attribute("arch").unwrap_or("").into(),
                status: status.attribute("code").unwrap_or("unknown").into(),
                details: status
                    .children()
                    .find(|n| n.has_tag_name("details"))
                    .and_then(|n| n.text())
                    .map(str::to_owned),
            });
        }
    }
    config::save(&area.directory().join("remote-result.toml"), row)?;
    Ok(())
}
