// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Package scaffold creation and local source-directory checks.

use crate::{check::metadata::Field, file_output, spec_metadata};
use clap::{Args, ValueEnum};
use minijinja::{Environment, UndefinedBehavior, context};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Args)]
pub(crate) struct Options {
    /// Name of the new package.
    #[arg(value_name = "NAME")]
    name: String,
    /// Existing output directory; defaults to the current directory.
    #[arg(long, value_name = "DIR", default_value = ".")]
    dir: PathBuf,
    /// Local SPEC directory to check for an existing NAME entry.
    #[arg(long, value_name = "DIR")]
    specs_dir: Option<PathBuf>,
    /// Amount of guidance in the template.
    #[arg(long, value_enum, default_value = "standard")]
    comments: Comments,
    #[command(flatten, next_help_heading = "Output options")]
    output: file_output::OutputActionOptions,
}

#[derive(Clone, Copy, ValueEnum)]
enum Comments {
    Standard,
    Full,
}

pub(crate) fn run(options: &Options) -> Result<(), InitError> {
    Field::Name
        .validate(&options.name)
        .map_err(InitError::Invalid)?;
    require_directory(&options.dir)?;
    let directory = fs::canonicalize(&options.dir).map_err(|source| InitError::Directory {
        path: options.dir.clone(),
        source,
    })?;
    let specs = match &options.specs_dir {
        Some(path) => {
            require_directory(path)?;
            Some(path.clone())
        }
        None => find_specs_directory(&directory)?,
    };
    if let Some(specs) = specs {
        // Read the working directory, including untracked entries; no Git or RPM index.
        let existing = specs.join(&options.name);
        match fs::symlink_metadata(&existing) {
            Ok(_) => {
                return Err(InitError::Invalid(format!(
                    "{} already exists; inspect the existing package before creating a new manifest",
                    existing.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(InitError::Directory {
                    path: existing,
                    source,
                });
            }
        }
    } else {
        warning(
            "no local SPECS directory found; package name availability was not checked (use --specs-dir DIR)",
        )?;
    }
    let year = match time::OffsetDateTime::now_local() {
        Ok(now) => now.year(),
        Err(_) => {
            warning("local time zone is unavailable; using the current UTC year")?;
            time::OffsetDateTime::now_utc().year()
        }
    }
    .to_string();
    let author = git_author(&directory);
    if author.is_none() {
        warning(
            "Git author is unavailable or unsuitable for a SPEC header; fill spec.contributors",
        )?;
    }
    let contents = render(&options.name, &year, author.as_deref(), options.comments)?;
    file_output::run(
        &directory.join(format!("{}.toml", options.name)),
        &contents,
        &options.output,
    )
    .map_err(InitError::Output)
}

fn require_directory(path: &Path) -> Result<(), InitError> {
    let metadata = fs::metadata(path).map_err(|source| InitError::Directory {
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(InitError::Invalid(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    // An unreadable directory must not be mistaken for an absent package.
    fs::read_dir(path).map_err(|source| InitError::Directory {
        path: path.to_owned(),
        source,
    })?;
    Ok(())
}

fn find_specs_directory(directory: &Path) -> Result<Option<PathBuf>, InitError> {
    for parent in directory.ancestors() {
        let path = parent.join("SPECS");
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                require_directory(&path)?;
                return Ok(Some(path));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(InitError::Directory { path, source }),
        }
    }
    Ok(None)
}

fn git_author(directory: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(directory)
        .args(["-c", "user.useConfigOnly=true", "var", "GIT_AUTHOR_IDENT"])
        .output()
        .ok()?;
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
}

fn render(
    name: &str,
    year: &str,
    author: Option<&str>,
    comments: Comments,
) -> Result<String, minijinja::Error> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.add_filter("toml", |value: String| {
        toml::Value::String(value).to_string()
    });
    env.add_template("init", include_str!("../templates/init.toml.j2"))?;
    env.get_template("init")?
        .render(context!(name, year, author, full => matches!(comments, Comments::Full)))
}

fn warning(message: &str) -> Result<(), InitError> {
    writeln!(io::stderr().lock(), "warning: {message}").map_err(InitError::Stderr)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InitError {
    #[error("{0}")]
    Invalid(String),
    #[error("cannot read directory {}: {source}", .path.display())]
    Directory { path: PathBuf, source: io::Error },
    #[error("failed to render manifest template: {0}")]
    Template(#[from] minijinja::Error),
    #[error("{0}")]
    Output(#[source] file_output::OutputError),
    #[error("failed to write warning: {0}")]
    Stderr(io::Error),
}
