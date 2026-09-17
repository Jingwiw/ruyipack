// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Manifest selection and checked SPEC generation.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use crate::{file_output, render, utf8_file};

/// Generates the SPEC owned by one manifest.
pub(crate) fn run(
    requested_name: &str,
    manifest_path: Option<&Path>,
    output: &file_output::OutputOptions,
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
    let rendered = render::run(&manifest_source).map_err(|source| GenerateError::Render {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    if requested_name != rendered.name {
        return Err(GenerateError::ManifestNotForPackage {
            requested: requested_name.to_owned(),
            path: manifest_path.to_path_buf(),
        });
    }

    let default_target = manifest_path.with_file_name(format!("{}.spec", rendered.name));
    let target = output.path.as_deref().unwrap_or(&default_target);
    rendered
        .report
        .write_human(target, &mut io::stderr().lock())
        .map_err(GenerateError::Stderr)?;
    if !rendered.report.is_success() {
        return Err(GenerateError::CheckFailed);
    }

    if !output.action.stdout && !output.action.diff {
        if manifest_path == target {
            return Err(GenerateError::InputIsTarget(target.to_path_buf()));
        }
        match fs::symlink_metadata(target) {
            Ok(_) => reject_input_alias(manifest_path, target)?,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(GenerateError::Read {
                    path: target.to_path_buf(),
                    source,
                });
            }
        }
    }
    file_output::run(
        target,
        &rendered.contents,
        &output.action,
        file_output::ConflictHint::WithOutputPath,
    )
    .map_err(GenerateError::Output)
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

#[derive(Debug, thiserror::Error)]
pub(crate) enum GenerateError {
    #[error("{0}")]
    Input(#[source] utf8_file::Utf8FileError),
    #[error("failed to generate SPEC from {}: {source}", .path.display())]
    Render {
        path: PathBuf,
        #[source]
        source: render::RenderError,
    },
    #[error("{0}")]
    Output(#[source] file_output::OutputError),
    #[error("failed to read {}: {source}", .path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("manifest path conflicts with generated target {}", .0.display())]
    InputIsTarget(PathBuf),
    #[error("no manifest for requested package {requested:?} was found at {}", .path.display())]
    ManifestNotForPackage { requested: String, path: PathBuf },
    #[error("package name {0:?} is not a valid selector; NAME must be one filename component")]
    InvalidPackageSelector(String),
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
    #[error("generated SPEC failed the selected static checks")]
    CheckFailed,
}
