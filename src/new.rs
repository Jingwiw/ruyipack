// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Package authoring scaffolds in named Git development areas.

use crate::{file_output, output_cli, spec_metadata, workspace};
use askama::Template;
use clap::{Args, ValueEnum};
use std::{
    io::{self, Write},
    path::Path,
    process::Command,
    time::Duration,
};

#[derive(Args)]
#[command(
    after_help = "Creates work/WORK/checkout from the recipe repository's committed main.\n\
The authoring scaffold is work/WORK/PKG.toml, outside Git.\n\
Existing SPEC files are kept; this is not a reverse conversion of their contents.\n\
Preview with --stdout or --diff without creating a development area."
)]
pub(crate) struct Options {
    /// Development area to create or reuse; also the default package name.
    #[arg(value_name = "WORK")]
    name: String,
    /// Actual package name; binds a new area and cannot rebind an existing one.
    #[arg(long, value_name = "PKG")]
    pkgname: Option<String>,
    /// Build system whose defaults and requirements seed the template.
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(crate::profile::buildsystems::systems()))]
    build_system: Option<String>,
    /// Amount of guidance in the template.
    #[arg(long, value_enum, default_value = "standard")]
    comments: Comments,
    #[command(flatten, next_help_heading = "Output options")]
    output: output_cli::OutputActionOptions,
}

#[derive(Clone, Copy, ValueEnum)]
enum Comments {
    Standard,
    Full,
}

pub(crate) fn run(options: &Options) -> Result<(), NewError> {
    let workspace = workspace::discover().map_err(NewError::Workspace)?;
    let preview = options.output.stdout || options.output.diff;
    let mut development = workspace
        .development(&options.name, options.pkgname.as_deref(), preview)
        .map_err(NewError::Workspace)?;
    let year = if let Ok(now) = time::OffsetDateTime::now_local() {
        now.year()
    } else {
        warning("local time zone is unavailable; using the current UTC year")?;
        time::OffsetDateTime::now_utc().year()
    }
    .to_string();
    let author = git_author(&development.author_directory()).map_err(NewError::Workspace)?;
    if author.is_none() {
        warning(
            "Git author is unavailable or unsuitable for a SPEC header; fill spec.contributors",
        )?;
    }
    let contents = render(options, development.package(), &year, author.as_deref())?;
    if !preview {
        development.create().map_err(NewError::Workspace)?;
    }
    let manifest = development.manifest();
    options
        .output
        .emit(&manifest, &contents)
        .map_err(NewError::Output)?;
    // A skipped conflicting file or a copied sidecar is not the authoring input
    // requested here. Preview never changes the selection or allocates a lock.
    if !preview && fs_err::read(&manifest).map_err(NewError::Workspace)? == contents.as_bytes() {
        development
            .select_input(workspace::GenerationInput::Authoring)
            .map_err(NewError::Workspace)?;
    }
    Ok(())
}

fn git_author(directory: &Path) -> io::Result<Option<String>> {
    let mut command = Command::new("git");
    command.current_dir(directory).args([
        "-c",
        "user.useConfigOnly=true",
        "var",
        "GIT_AUTHOR_IDENT",
    ]);
    let output =
        match crate::host_process::capture(&mut command, Duration::from_secs(30), 64 * 1024) {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(_) => return Ok(None),
        };
    Ok((|| {
        if !output.status.success() {
            return None;
        }
        let ident = std::str::from_utf8(&output.stdout).ok()?.trim_end();
        let (name_email, _) = ident.rsplit_once("> ")?;
        let (name, email) = name_email.rsplit_once(" <")?;
        if name.is_empty() || email.is_empty() {
            return None;
        }
        let author = format!("{name_email}>");
        spec_metadata::validate_contributor(&author).ok()?;
        Some(author)
    })())
}

#[derive(Template)]
#[template(path = "new.toml.j2", escape = "none")]
struct Scaffold<'a> {
    name: &'a str,
    year: &'a str,
    author: Option<&'a str>,
    system: Option<&'a str>,
    requirements: &'a [String],
    stages: &'a [crate::profile::buildsystems::StageAction],
    full: bool,
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn render(
    options: &Options,
    name: &str,
    year: &str,
    author: Option<&str>,
) -> Result<String, askama::Error> {
    let system = options.build_system.as_deref();
    let contract = system.and_then(crate::profile::buildsystems::contract);
    let requirements = contract
        .map(|contract| contract.build_requires.as_slice())
        .unwrap_or_default();
    let stages = contract
        .map(|contract| contract.stages.as_slice())
        .unwrap_or_default();
    Scaffold {
        name,
        year,
        author,
        system,
        requirements,
        stages,
        full: matches!(options.comments, Comments::Full),
    }
    .render()
}

fn warning(message: &str) -> Result<(), NewError> {
    writeln!(io::stderr().lock(), "warning: {message}").map_err(NewError::Stderr)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum NewError {
    #[error("{0}")]
    Workspace(#[source] io::Error),
    #[error("failed to render manifest template: {0}")]
    Template(#[from] askama::Error),
    #[error("{0}")]
    Output(#[source] file_output::OutputError),
    #[error("failed to write warning: {0}")]
    Stderr(io::Error),
}
