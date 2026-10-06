// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Package authoring inputs in named development areas.

mod spec_import;

use crate::{file_output, output_cli, workspace};
use askama::Template;
use clap::{Args, ValueEnum};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Args)]
#[command(
    after_help = "Creates a local recipe directory without requiring Git. Existing packages are copied from committed main by edit/open/build.\n\
The authoring TOML is outside the recipe directory and Git.\n\
--from-toml copies an existing authoring TOML unchanged; its package.name supplies the package binding.\n\
--from-dir imports a package directory; --from-spec imports only one SPEC and editable fields while retaining the original SPEC.\n\
Preview with --stdout or --diff without creating a development area."
)]
pub(crate) struct Options {
    /// Development area name; the default package name only when no source supplies one.
    #[arg(value_name = "WORK")]
    name: String,
    /// Override the package directory name, not the source selection or SPEC Name.
    #[arg(long, value_name = "PKG")]
    pkgname: Option<String>,
    /// Import an authoring TOML verbatim; gen validates incomplete package facts.
    #[arg(long = "from-toml", group = "source", value_name = "MANIFEST", value_hint = clap::ValueHint::FilePath,
        conflicts_with_all = ["build_system", "comments"])]
    from_toml: Option<PathBuf>,
    /// Import only a SPEC; retain unmapped SPEC text.
    #[arg(long = "from-spec", group = "source", value_name = "PATH", value_hint = clap::ValueHint::FilePath,
        conflicts_with_all = ["build_system", "comments", "force", "skip_existing"])]
    from_spec: Option<PathBuf>,
    /// Copy a directory with exactly one top-level SPEC; use its filename as the package name.
    #[arg(long = "from-dir", group = "source", value_name = "DIR", value_hint = clap::ValueHint::DirPath,
        conflicts_with_all = ["build_system", "comments", "force", "skip_existing"])]
    from_dir: Option<PathBuf>,
    /// Build system whose defaults and requirements seed the template.
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(crate::profile::buildsystems::systems()))]
    build_system: Option<String>,
    /// Amount of guidance in the template.
    #[arg(long, value_enum, default_value = "standard")]
    comments: Comments,
    /// Print the authoring scaffold without creating a development area.
    #[arg(long, conflicts_with_all = ["diff", "force", "skip_existing"])]
    stdout: bool,
    /// Compare the scaffold without creating or changing files.
    #[arg(long, conflicts_with_all = ["force", "skip_existing"])]
    diff: bool,
    #[command(flatten, next_help_heading = "Output options")]
    output: output_cli::ConflictOptions,
}

#[derive(Clone, Copy, ValueEnum)]
enum Comments {
    Standard,
    Full,
}

pub(crate) fn run(options: &Options) -> Result<(), NewError> {
    let workspace = workspace::discover().map_err(NewError::Workspace)?;
    if let Some(path) = &options.from_spec {
        return spec_import::run(options, &workspace, path, false);
    }
    if let Some(directory) = &options.from_dir {
        let directory = fs_err::canonicalize(directory).map_err(NewError::Workspace)?;
        let spec = workspace::spec_in(&directory, None).map_err(NewError::Workspace)?;
        return spec_import::run(options, &workspace, &spec, true);
    }
    let imported = options.from_toml.as_deref().map(import).transpose()?;
    let package = imported.as_ref().and_then(|(_, name)| name.as_deref());
    if let (Some(explicit), Some(imported)) = (options.pkgname.as_deref(), package)
        && explicit != imported
    {
        return Err(NewError::Input(format!(
            "package.name {imported:?} conflicts with --pkgname {explicit:?}"
        )));
    }
    let preview = options.stdout || options.diff;
    let mut development = workspace
        .new_development(
            &options.name,
            options.pkgname.as_deref().or(package),
            preview,
            workspace::DevelopmentKind::Local,
        )
        .map_err(NewError::Workspace)?;
    let contents = match imported {
        Some((contents, _)) => contents,
        None => scaffold(
            options,
            development.package(),
            workspace.author().map_err(NewError::Workspace)?,
        )?,
    };
    if !preview {
        development.create().map_err(NewError::Workspace)?;
    }
    let manifest = development.manifest();
    if options.stdout || options.diff {
        let text = if options.diff {
            file_output::target_diff(&manifest, &contents).map_err(NewError::Output)?
        } else {
            contents
        };
        io::stdout()
            .lock()
            .write_all(text.as_bytes())
            .map_err(|e| NewError::Output(file_output::OutputError::Stdout(e)))?;
    } else {
        let outcome = options
            .output
            .publish(&manifest, &contents)
            .map_err(NewError::Output)?;
        if matches!(outcome, file_output::EditOutcome::Written(ref path) if path != &manifest) {
            output_cli::stderr()
                .message(
                    output_cli::HumanLevel::Info,
                    Some(Path::new(&options.name)),
                    format_args!("copy is not bound to this WORK; authoring input is unchanged"),
                )
                .map_err(NewError::Stderr)?;
            return Ok(());
        }
    }
    if !preview {
        show_authoring(&options.name, &manifest)?;
    }
    Ok(())
}

fn show_authoring(work: &str, path: &Path) -> Result<(), NewError> {
    let mut output = output_cli::stderr();
    output
        .message(
            output_cli::HumanLevel::Info,
            Some(Path::new(work)),
            format_args!("authoring: {}", output_cli::human_path(path).display()),
        )
        .map_err(NewError::Stderr)?;
    output
        .message(
            output_cli::HumanLevel::Info,
            Some(Path::new(work)),
            format_args!(
                "next: ruyipack open {0} --authoring; ruyipack gen {0} --diff",
                shell_words::quote(work)
            ),
        )
        .map_err(NewError::Stderr)
}

fn import(path: &Path) -> Result<(String, Option<String>), NewError> {
    let contents = crate::utf8_file::read(path).map_err(|e| NewError::Input(e.to_string()))?;
    let document: toml::Table = toml::from_str(&contents)
        .map_err(|e| NewError::Input(format!("{}: {e}", path.display())))?;
    let name = (|| {
        let Some(package) = document.get("package") else {
            return Ok(None);
        };
        let package = package.as_table().ok_or("package must be a table")?;
        package
            .get("name")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("package.name must be a string")
            })
            .transpose()
    })()
    .map_err(|e: &str| NewError::Input(format!("{}: {e}", path.display())))?;
    Ok((contents, name))
}

fn scaffold(options: &Options, name: &str, author: Option<&str>) -> Result<String, NewError> {
    let year = if let Ok(now) = time::OffsetDateTime::now_local() {
        now.year()
    } else {
        warning("local time zone is unavailable; using the current UTC year")?;
        time::OffsetDateTime::now_utc().year()
    }
    .to_string();
    Ok(render(options, name, &year, author)?)
}

#[derive(Template)]
#[template(path = "new.toml.j2", escape = "none")]
struct Scaffold<'a> {
    name: &'a str,
    year: &'a str,
    author: Option<&'a str>,
    system: Option<&'a str>,
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
    let stages = contract
        .map(|contract| contract.stages.as_slice())
        .unwrap_or_default();
    Scaffold {
        name,
        year,
        author,
        system,
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
    Input(String),
    #[error("{0}")]
    Workspace(#[source] io::Error),
    #[error("failed to render manifest template: {0}")]
    Template(#[from] askama::Error),
    #[error("{0}")]
    Output(#[source] file_output::OutputError),
    #[error("failed to write warning: {0}")]
    Stderr(io::Error),
}
