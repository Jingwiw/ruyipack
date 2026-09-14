// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Manifest selection and SPEC preview output.

use std::{
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use crate::{render, utf8_file};

/// Prints the SPEC candidate owned by one manifest.
pub(crate) fn run(requested_name: &str, manifest_path: Option<&Path>) -> Result<(), GenerateError> {
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

    io::stdout()
        .lock()
        .write_all(rendered.contents.as_bytes())
        .map_err(GenerateError::Stdout)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GenerateError {
    #[error("{0}")]
    Input(#[source] utf8_file::Utf8FileError),
    #[error("{0}")]
    Render(#[source] render::RenderError),
    #[error("failed to write generated SPEC to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("no manifest for requested package {requested:?} was found at {}", .path.display())]
    ManifestNotForPackage { requested: String, path: PathBuf },
    #[error("package name {0:?} is not a valid selector; NAME must be one filename component")]
    InvalidPackageSelector(String),
    #[error("generated SPEC failed the selected static checks")]
    CheckFailed,
}
