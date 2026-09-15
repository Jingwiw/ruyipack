// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Safe materialization of generated package artifacts.

use std::{
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use crate::{render, utf8_file};

/// Generates the SPEC owned by one manifest.
pub(crate) fn run(
    requested_name: &str,
    manifest_path: Option<&Path>,
    force: bool,
    stdout: bool,
) -> Result<(), GenerateError> {
    // NAME selects a package; it never acts as an implicit manifest path.
    let mut name_components = Path::new(requested_name).components();
    let name_is_file_component = matches!(name_components.next(), Some(Component::Normal(_)))
        && name_components.next().is_none()
        && !requested_name.contains('/')
        && !requested_name.contains('\\');
    if !name_is_file_component {
        return Err(GenerateError::InvalidPackageSelector(
            requested_name.to_owned(),
        ));
    }

    let default_manifest;
    let manifest_path = match manifest_path {
        Some(path) => path,
        None => {
            default_manifest = PathBuf::from(format!("{requested_name}.toml"));
            &default_manifest
        }
    };

    let manifest_source = match utf8_file::read(manifest_path) {
        Ok(source) => source,
        Err(utf8_file::Utf8FileError::Read { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return Err(GenerateError::ManifestNotForPackage {
                requested: requested_name.to_owned(),
                path: manifest_path.to_path_buf(),
            });
        }
        Err(source) => return Err(GenerateError::Input(source)),
    };
    let rendered = render::run(&manifest_source).map_err(GenerateError::Render)?;
    if requested_name != rendered.name {
        return Err(GenerateError::ManifestNotForPackage {
            requested: requested_name.to_owned(),
            path: manifest_path.to_path_buf(),
        });
    }

    let target = manifest_path.with_file_name(format!("{}.spec", rendered.name));
    rendered.report.print_human(&target);
    if !rendered.report.is_success() {
        return Err(GenerateError::CheckFailed);
    }

    if stdout {
        io::stdout()
            .lock()
            .write_all(rendered.contents.as_bytes())
            .map_err(GenerateError::Stdout)?;
        return Ok(());
    }

    if manifest_path == target {
        return Err(GenerateError::InputIsTarget(target));
    }

    match fs::read(&target) {
        Ok(existing) => {
            reject_input_alias(manifest_path, &target)?;
            if existing == rendered.contents.as_bytes() {
                Ok(())
            } else if force {
                write_atomically(&target, rendered.contents.as_bytes(), true)
            } else {
                Err(GenerateError::DifferentTarget(target))
            }
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            write_atomically(&target, rendered.contents.as_bytes(), false)
        }
        Err(source) => Err(GenerateError::Read {
            path: target,
            source,
        }),
    }
}

/// Rejects an existing target that resolves to the manifest itself.
fn reject_input_alias(manifest_path: &Path, target: &Path) -> Result<(), GenerateError> {
    let manifest = fs::canonicalize(manifest_path).map_err(|source| GenerateError::Read {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    let target_path = fs::canonicalize(target).map_err(|source| GenerateError::Read {
        path: target.to_path_buf(),
        source,
    })?;

    if manifest == target_path {
        Err(GenerateError::InputIsTarget(target.to_path_buf()))
    } else {
        Ok(())
    }
}

/// Publishes staged bytes with explicit overwrite permission.
fn write_atomically(path: &Path, contents: &[u8], replace: bool) -> Result<(), GenerateError> {
    let write_error = |source| GenerateError::Write {
        path: path.to_path_buf(),
        source,
    };
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(".ruyipack.");
    // Match ordinary file creation permissions, subject to the caller's umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut file = builder.tempfile_in(parent).map_err(write_error)?;
    file.write_all(contents)
        .and_then(|()| file.as_file().sync_all())
        .map_err(write_error)?;
    let result = if replace {
        file.persist(path)
    } else {
        file.persist_noclobber(path)
    };
    match result {
        Ok(_) => Ok(()),
        Err(error) if !replace && error.error.kind() == io::ErrorKind::AlreadyExists => {
            match fs::read(path).map_err(|source| GenerateError::Read {
                path: path.to_path_buf(),
                source,
            })? {
                existing if existing == contents => Ok(()),
                _ => Err(GenerateError::DifferentTarget(path.to_path_buf())),
            }
        }
        Err(error) => Err(write_error(error.error)),
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GenerateError {
    #[error("{0}")]
    Input(#[source] utf8_file::Utf8FileError),
    #[error("{0}")]
    Render(#[source] render::RenderError),
    #[error("failed to write generated SPEC to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("failed to read {}: {source}", .path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to write {}: {source}", .path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("{} already exists with different content; use `--force` to replace it or `--stdout` to inspect the candidate", .0.display())]
    DifferentTarget(PathBuf),
    #[error("manifest path conflicts with generated target {}", .0.display())]
    InputIsTarget(PathBuf),
    #[error("no manifest for requested package {requested:?} was found at {}", .path.display())]
    ManifestNotForPackage { requested: String, path: PathBuf },
    #[error("package name {0:?} is not a valid selector; NAME must be one filename component")]
    InvalidPackageSelector(String),
    #[error("generated SPEC failed the selected static checks")]
    CheckFailed,
}
