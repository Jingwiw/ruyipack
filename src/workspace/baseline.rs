// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! File identities shared by import baselines and scoped Git publication.

use super::invalid;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::{Component, Path},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct File {
    pub sha256: String,
    pub executable: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Baseline {
    pub files: Files,
    // Only a fresh local/template import may create an absent target package.
    // Once based on a repository, its removal upstream is a conflict, not an add.
    pub allow_create: bool,
}

pub(crate) type Files = BTreeMap<String, File>;

pub(super) fn relative(value: &str) -> io::Result<&Path> {
    let path = Path::new(value);
    if value.is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(s) if !s.eq_ignore_ascii_case(".git")))
    {
        return Err(invalid(format!("invalid package path: {value:?}")));
    }
    Ok(path)
}

pub(crate) fn read(root: &Path) -> io::Result<Files> {
    let mut files = Files::new();
    if root.try_exists()? {
        collect(root, root, &mut files)?;
    }
    Ok(files)
}

fn collect(root: &Path, path: &Path, files: &mut Files) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(root, &entry?.path(), files)?;
        }
    } else {
        let name = path
            .strip_prefix(root)
            .expect("descendant")
            .to_str()
            .ok_or_else(|| invalid("package filenames must be UTF-8"))?;
        relative(name)?;
        let content = crate::file_digest::read(path).map_err(io::Error::other)?;
        let executable = crate::file_digest::executable(&meta);
        files.insert(
            name.to_owned(),
            File {
                sha256: content.sha256,
                executable,
            },
        );
    }
    Ok(())
}

pub(crate) fn load<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    toml::from_str(&crate::utf8_file::read(path).map_err(io::Error::other)?)
        .map_err(|e| invalid(format!("{}: {e}", path.display())))
}

pub(crate) fn save(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let text = toml::to_string(value).map_err(io::Error::other)?;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("state parent"))?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
