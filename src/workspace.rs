// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Workspace roots and user-owned configuration, independent of build tools.

mod checkout;
mod development;
mod input;

pub(crate) use development::{Development, GenerationInput};
pub(crate) use input::SpecOptions;

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
    #[arg(value_name = "PATH", default_value = ".")]
    path: PathBuf,
    /// Clone a recipe Git repository after initialization; omit for offline initialization.
    #[arg(long, value_name = "URL")]
    clone: Option<OsString>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    recipes: PathBuf,
    work: PathBuf,
    specs: PathBuf,
}

pub(crate) struct Workspace {
    recipes: PathBuf,
    work: PathBuf,
    specs: PathBuf,
}

/// Stop at the nearest marker, including an interrupted or invalid workspace.
pub(crate) fn discover() -> io::Result<Workspace> {
    let current = std::env::current_dir()?;
    for root in current.ancestors() {
        match fs::symlink_metadata(root.join(".ruyiconfig")) {
            Ok(_) => return load(root),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Err(invalid(
        "not in a RuyiPack workspace; run `ruyipack init` in an empty directory first",
    ))
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
    fs::write(marker.join("config.toml"), DEFAULT_CONFIG)?;
    crate::environment::write(&marker.join("build"))?;
    writeln!(
        io::stdout().lock(),
        "Initialized RuyiPack workspace at {}",
        root.display()
    )?;
    if let Some(url) = &options.clone {
        let workspace = load(&root)?;
        clone_recipes(&root, &workspace.recipes, url)?;
    }
    Ok(())
}

fn clone_recipes(root: &Path, recipes: &Path, url: &OsStr) -> io::Result<()> {
    checkout::clone_repository(recipes, url).map_err(|error| {
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
        recipes,
        work,
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
