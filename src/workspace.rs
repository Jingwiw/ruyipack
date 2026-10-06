// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Workspace roots and user-owned configuration, independent of build tools.

mod baseline;
pub(crate) mod commit;
mod commit_scope;
pub(crate) mod delete;
mod development;
mod git;
mod input;
pub(crate) mod pr;
mod recipe;

pub(crate) use development::{Development, DevelopmentKind};
pub(crate) use input::SpecOptions;
pub(crate) use recipe::spec_in;

use clap::Args;
use fs_err as fs;
use serde::Deserialize;
use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

const DEFAULT_CONFIG: &str = "recipes = \"openruyi\"\nwork = \"work\"\nspecs = \"SPECS\"\n";

#[derive(Args)]
pub(crate) struct Options {
    /// Empty or nonexistent directory to initialize.
    #[arg(value_name = "PATH", default_value = ".", value_hint = clap::ValueHint::DirPath)]
    path: PathBuf,
    /// Clone a recipe Git repository after initialization; omit for offline initialization.
    #[arg(long, value_name = "URL")]
    clone: Option<OsString>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    repology: std::collections::BTreeMap<String, String>,
    recipes: PathBuf,
    work: PathBuf,
    specs: PathBuf,
    editor: Option<String>,
    author: Option<String>,
}

pub(crate) struct Workspace {
    repology: std::collections::BTreeMap<String, String>,
    root: PathBuf,
    recipes: PathBuf,
    work: PathBuf,
    specs: PathBuf,
    editor: Option<String>,
    author: Option<String>,
}

impl Workspace {
    pub(crate) fn configuration(&self) -> PathBuf {
        self.root.join(".ruyiconfig")
    }

    pub(crate) fn repology_project(&self, package: &str) -> Option<&str> {
        self.repology.get(package).map(String::as_str)
    }

    /// User-owned packaging identity; unrelated workspace operations need not validate it.
    pub(crate) fn author(&self) -> io::Result<Option<&str>> {
        let author = self.author.as_deref().filter(|value| valid_author(value));
        if author.is_none() {
            crate::output_cli::stderr().message(
                crate::output_cli::HumanLevel::Warn,
                Some(&self.root.join(".ruyiconfig/config.toml")),
                format_args!("author is missing or invalid; set author = \"Name <email>\". Package authors: spec.contributors"),
            )?;
        }
        Ok(author)
    }

    pub(crate) fn recipes(&self) -> &Path {
        &self.recipes
    }

    pub(crate) fn editor(&self) -> Option<&str> {
        self.editor.as_deref()
    }

    pub(crate) fn build_config(&self) -> PathBuf {
        self.root.join(".ruyiconfig/build/compose.yaml")
    }
}

pub(crate) fn discover() -> io::Result<Workspace> {
    discover_optional()?.ok_or_else(|| {
        invalid("not in a RuyiPack workspace; run `ruyipack init` in an empty directory first")
    })
}

/// Explicit file operations may run outside a workspace; an invalid marker still fails.
pub(crate) fn discover_optional() -> io::Result<Option<Workspace>> {
    let current = std::env::current_dir()?;
    for root in current.ancestors() {
        match fs::symlink_metadata(root.join(".ruyiconfig")) {
            Ok(_) => return load(root).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

pub(crate) fn run(options: &Options) -> io::Result<()> {
    match fs::symlink_metadata(&options.path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(invalid(format!(
                "{} must be a directory, not a symlink",
                options.path.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(&options.path)?;
        }
        Err(error) => return Err(error),
    }
    let root = fs::canonicalize(&options.path)?;
    let marker = root.join(".ruyiconfig");
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            let mut stderr = io::stderr().lock();
            writeln!(
                stderr,
                "warning: {} is already initialized; existing files were not changed",
                root.display()
            )?;
            let result = if metadata.is_dir() {
                load(&root)
            } else {
                Err(invalid(".ruyiconfig must be a directory, not a symlink"))
            };
            return match result {
                Ok(workspace) => match &options.clone {
                    Some(url) => clone_recipes(&root, &workspace.recipes, url),
                    None => Ok(()),
                },
                Err(error) if options.clone.is_some() => Err(invalid(format!(
                    "workspace configuration is invalid: {error}; recipe clone was not attempted"
                ))),
                Err(error) => writeln!(
                    stderr,
                    "warning: workspace configuration is invalid: {error}"
                ),
            };
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if fs::read_dir(&root)?.next().transpose()?.is_some() {
        return Err(invalid(format!(
            "{} must be completely empty before initialization",
            root.display()
        )));
    }
    // Exclusive creation arbitrates concurrent init. Leave partial state on failure;
    // retry must warn rather than overwrite files or erase an interrupted workspace.
    fs::create_dir(&marker)?;
    let author = git_author(&root)?.unwrap_or_default();
    fs::write(
        marker.join("config.toml"),
        format!("{DEFAULT_CONFIG}author = {}\n", toml::Value::String(author)),
    )?;
    fs::write(marker.join("commit-ignore.toml"), commit_scope::DEFAULT)?;
    fs::write(marker.join("pr.md"), include_bytes!("../templates/pr.md"))?;
    let workspace = load(&root)?;
    workspace.author()?;
    crate::environment::write(&marker.join("build"))?;
    writeln!(
        io::stdout().lock(),
        "Initialized RuyiPack workspace at {}",
        root.display()
    )?;
    if let Some(url) = &options.clone {
        clone_recipes(&root, &workspace.recipes, url)?;
    }
    Ok(())
}

fn clone_recipes(root: &Path, recipes: &Path, url: &OsStr) -> io::Result<()> {
    git::clone_repository(recipes, url).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "workspace initialization succeeded at {}; recipe clone failed at {}: {error}; workspace configuration and any clone files were retained",
                root.display(),
                recipes.display()
            ),
        )
    })?;
    writeln!(
        io::stdout().lock(),
        "Cloned recipe repository to {}",
        recipes.display()
    )
}

fn load(root: &Path) -> io::Result<Workspace> {
    let marker = root.join(".ruyiconfig");
    if !fs::symlink_metadata(&marker)?.is_dir() {
        return Err(invalid(format!(
            "{} must be a directory, not a symlink",
            marker.display()
        )));
    }
    let path = root.join(".ruyiconfig/config.toml");
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid(format!(
            "{} must be a regular file",
            path.display()
        )));
    }
    let config: Config = toml::from_str(&fs::read_to_string(&path)?)
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
    relative(&config.work, "work")?;
    relative(&config.specs, "specs")?;
    if config.recipes.as_os_str().is_empty() {
        return Err(invalid("recipes must not be empty"));
    }
    let work = directory(root, &config.work, false)?;
    let recipes = directory(root, &config.recipes, true)?;
    directory(&recipes, &config.specs, false)?;
    for (left, right) in [(&work, &recipes), (&work, &marker), (&recipes, &marker)] {
        if left.starts_with(right) || right.starts_with(left) {
            return Err(invalid(format!(
                "configured paths must not overlap: {} and {}",
                left.display(),
                right.display()
            )));
        }
    }
    Ok(Workspace {
        repology: config.repology,
        root: root.to_path_buf(),
        recipes,
        work,
        editor: config.editor,
        author: config.author,
        specs: config
            .specs
            .components()
            .filter(|part| matches!(part, Component::Normal(_)))
            .collect(),
    })
}

fn relative(path: &Path, field: &str) -> io::Result<()> {
    if !path
        .components()
        .any(|part| matches!(part, Component::Normal(_)))
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(invalid(format!(
            "{field} must be a nonempty relative path without '..'"
        )));
    }
    Ok(())
}

// Resolve existing ancestors without requiring the recipe or work directory to exist.
// Resolve symlinks before '..' so overlap checks use filesystem identities.
fn directory(root: &Path, path: &Path, allow_symlinks: bool) -> io::Result<PathBuf> {
    let mut resolved = if path.is_absolute() {
        PathBuf::new()
    } else {
        root.to_path_buf()
    };
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(_) => {
                resolved.push(part.as_os_str());
                match fs::symlink_metadata(&resolved) {
                    Ok(metadata) => {
                        if !(metadata.is_dir() || allow_symlinks && metadata.is_symlink()) {
                            return Err(invalid(format!(
                                "{} must be a directory{}",
                                resolved.display(),
                                if allow_symlinks {
                                    ""
                                } else {
                                    ", not a symlink"
                                }
                            )));
                        }
                        resolved = fs::canonicalize(&resolved)?;
                        if !fs::metadata(&resolved)?.is_dir() {
                            return Err(invalid(format!(
                                "{} must be a directory",
                                resolved.display()
                            )));
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Component::Prefix(_) | Component::RootDir => resolved.push(part.as_os_str()),
        }
    }
    Ok(resolved)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

// Read global identity only at init, not repository identity or GIT_AUTHOR_* overrides.
fn git_author(root: &Path) -> io::Result<Option<String>> {
    let mut values = Vec::new();
    for field in ["user.name", "user.email"] {
        let output = match crate::host_process::capture(
            std::process::Command::new("git").current_dir(root).args([
                "config",
                "--global",
                "--includes",
                "--get",
                field,
            ]),
            std::time::Duration::from_secs(30),
            64 * 1024,
        ) {
            Ok(output) if output.status.success() => output,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            _ => return Ok(None),
        };
        let Ok(value) = String::from_utf8(output.stdout) else {
            return Ok(None);
        };
        values.push(value.trim_end_matches('\n').to_owned());
    }
    let author = format!("{} <{}>", values[0], values[1]);
    Ok(valid_author(&author).then_some(author))
}

fn valid_author(value: &str) -> bool {
    crate::spec_metadata::validate_contributor(value).is_ok()
        && value
            .strip_suffix('>')
            .and_then(|value| value.rsplit_once(" <"))
            .is_some_and(|(name, email)| !name.trim().is_empty() && !email.trim().is_empty())
}

pub(crate) fn save_baseline(work: &Path, package: &Path) -> io::Result<()> {
    baseline::save(
        &work.join("baseline.toml"),
        &baseline::Baseline {
            files: baseline::read(package)?,
            allow_create: true,
        },
    )
}
