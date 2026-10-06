// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! One read-only SPEC selection for CLI consumers; WORK keeps its operation lock.

use super::{Development, discover, invalid};
use clap::Args;
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub(crate) struct SpecOptions {
    /// Development area; first use binds the same package name unless --pkgname is given.
    #[arg(value_name = "WORK", conflicts_with = "spec", value_parser = clap::builder::NonEmptyStringValueParser::new())]
    pub(crate) work: Option<String>,
    /// Read an explicit SPEC instead of a development area.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["work", "pkgname"])]
    pub(crate) spec: Option<PathBuf>,
    /// Bind a new development area to this package; existing bindings cannot change.
    #[arg(long, value_name = "PKG", requires = "work")]
    pub(crate) pkgname: Option<String>,
}

pub(crate) struct SpecInput {
    pub(crate) path: PathBuf,
    pub(crate) source: String,
    /// Present only when reading an immutable main commit without a materialize.
    pub(crate) revision: Option<String>,
    development: Option<Development>,
    canonical: Option<PathBuf>,
}

impl SpecOptions {
    pub(crate) fn display(&self) -> String {
        self.spec
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| self.work.clone())
            .unwrap_or_else(|| "<no input>".into())
    }

    pub(crate) fn resolve(&self) -> io::Result<SpecInput> {
        self.resolve_with_materials(false)
    }

    pub(crate) fn resolve_materials(&self) -> io::Result<SpecInput> {
        self.resolve_with_materials(true)
    }

    fn resolve_with_materials(&self, materialize: bool) -> io::Result<SpecInput> {
        if let Some(path) = &self.spec {
            let path = path.clone();
            let source = crate::utf8_file::read(&path).map_err(io::Error::other)?;
            let canonical = fs_err::canonicalize(&path)?;
            return Ok(SpecInput {
                path,
                source,
                revision: None,
                development: None,
                canonical: Some(canonical),
            });
        }
        let work = self
            .work
            .as_deref()
            .ok_or_else(|| invalid("select WORK or --spec PATH"))?;
        let mut development = discover()?.development(work, self.pkgname.as_deref(), false)?;
        if materialize {
            development.create()?;
        }
        let (path, source, revision) = development.source()?;
        development.ensure_work()?;
        let canonical = revision
            .is_none()
            .then(|| fs_err::canonicalize(&path))
            .transpose()?;
        Ok(SpecInput {
            path,
            source,
            revision,
            development: Some(development),
            canonical,
        })
    }
}

impl SpecInput {
    pub(crate) fn development(&self) -> Option<&Development> {
        self.development.as_ref()
    }

    pub(crate) fn sources(&self) -> Option<PathBuf> {
        self.development.as_ref().map(Development::sources)
    }

    pub(crate) fn directory(&self) -> &Path {
        self.canonical
            .as_ref()
            .unwrap_or(&self.path)
            .parent()
            .expect("SPEC path has a parent")
    }

    pub(crate) fn is_unchanged(&self) -> io::Result<bool> {
        let Some(canonical) = &self.canonical else {
            // Git blob identity is immutable; a dirty recipe working copy is irrelevant.
            return Ok(true);
        };
        Ok(
            fs_err::canonicalize(&self.path).is_ok_and(|actual| actual == *canonical)
                && crate::utf8_file::is_unchanged(canonical, &self.source)
                    .map_err(io::Error::other)?,
        )
    }
}
