// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! File publication and conflict-time source and destination checks.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use crate::utf8_file;

#[derive(Clone, Copy)]
pub(crate) enum ConflictAction {
    Overwrite,
    Diff,
    Copy,
    Skip,
}

/// Publishes validated text without silently replacing different content.
/// A selection authorizes an action, not stale bytes: revalidation stays here.
pub(crate) fn publish(
    diff: &mut impl Write,
    path: &Path,
    contents: &str,
    mut choose: impl FnMut(&Path) -> Result<ConflictAction, OutputError>,
) -> Result<EditOutcome, OutputError> {
    let existing = read_optional_target(path)?;
    publish_one(
        diff,
        path,
        contents,
        existing.as_deref(),
        None,
        &mut choose,
        || Ok(()),
    )
}

/// One validated candidate and the exact source bytes from which it was prepared.
pub(crate) struct EditFile<'a> {
    pub(crate) source_path: &'a Path,
    pub(crate) original: &'a str,
    pub(crate) contents: &'a str,
}

/// Observed result for one edit destination.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditOutcome {
    Written(PathBuf),
    Unchanged(PathBuf),
    Skipped(PathBuf),
}

/// Publishes checked candidates, retaining exact paths on partial failure.
pub(crate) fn run_edits(
    diff: &mut impl Write,
    files: &[EditFile<'_>],
    output: Option<&Path>,
    mut choose: impl FnMut(&Path) -> Result<ConflictAction, OutputError>,
) -> Result<Vec<EditOutcome>, OutputError> {
    let mut outcomes = Vec::new();
    match run_edit_batch(diff, files, output, &mut choose, &mut outcomes) {
        Ok(()) => Ok(outcomes),
        Err(source) => {
            let written = outcomes
                .into_iter()
                .filter_map(|outcome| match outcome {
                    EditOutcome::Written(path) => Some(path),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if written.is_empty() {
                Err(source)
            } else {
                Err(OutputError::Partial {
                    written,
                    source: Box::new(source),
                })
            }
        }
    }
}

fn run_edit_batch(
    diff: &mut impl Write,
    files: &[EditFile<'_>],
    output: Option<&Path>,
    choose: &mut impl FnMut(&Path) -> Result<ConflictAction, OutputError>,
    outcomes: &mut Vec<EditOutcome>,
) -> Result<(), OutputError> {
    check_sources(files, outcomes)?;
    if files.is_empty() {
        return Ok(());
    }
    let targets = edit_targets(files, output)?;
    let existing = targets
        .iter()
        .map(|path| read_optional_target(path))
        .collect::<Result<Vec<_>, _>>()?;
    for ((file, target), existing) in files.iter().zip(&targets).zip(&existing) {
        let outcome = publish_one(
            diff,
            target,
            file.contents,
            existing.as_deref(),
            Some(file.source_path),
            &mut |path| {
                // Writing back to the source is the edit action, not a conflict.
                if target == file.source_path {
                    Ok(ConflictAction::Overwrite)
                } else {
                    choose(path)
                }
            },
            || check_sources(files, outcomes),
        )?;
        outcomes.push(outcome);
    }
    Ok(())
}

fn check_sources(files: &[EditFile<'_>], outcomes: &[EditOutcome]) -> Result<(), OutputError> {
    for (index, file) in files.iter().enumerate() {
        if !matches!(outcomes.get(index), Some(EditOutcome::Written(path)) if path == file.source_path)
            && !utf8_file::is_unchanged(file.source_path, file.original).map_err(|source| {
                OutputError::Read {
                    path: file.source_path.to_path_buf(),
                    source,
                }
            })?
        {
            return Err(OutputError::SourceChanged(file.source_path.to_path_buf()));
        }
    }
    Ok(())
}

/// Resolves destination parents while keeping the final path entry explicit.
fn edit_targets(
    files: &[EditFile<'_>],
    output: Option<&Path>,
) -> Result<Vec<PathBuf>, OutputError> {
    let mut sources: Vec<(PathBuf, fs::Metadata)> = Vec::new();
    for file in files {
        let path = fs::canonicalize(file.source_path).map_err(|source| OutputError::Read {
            path: file.source_path.to_path_buf(),
            source,
        })?;
        let metadata = fs::metadata(&path).map_err(|source| OutputError::Read {
            path: path.clone(),
            source,
        })?;
        if sources
            .iter()
            .any(|(other, data)| paths_alias(&path, Some(&metadata), other, Some(data)))
        {
            return Err(OutputError::EditLayout(format!(
                "duplicate or aliased source {}",
                path.display()
            )));
        }
        sources.push((path, metadata));
    }
    let Some(output) = output else {
        return Ok(sources.into_iter().map(|(path, _)| path).collect());
    };
    let [(source, data)] = sources.as_slice() else {
        return Err(OutputError::EditLayout(
            "--output requires exactly one file".into(),
        ));
    };
    let path = output_path(output).map_err(|source| OutputError::Read {
        path: output.to_path_buf(),
        source,
    })?;
    let metadata = target_metadata(&path)?;
    if path != *source && paths_alias(&path, metadata.as_ref(), source, Some(data)) {
        return Err(OutputError::EditLayout(format!(
            "target {} aliases source {}",
            path.display(),
            source.display()
        )));
    }
    Ok(vec![path])
}

/// Resolve the parent, not the final entry: a destination may not exist yet.
pub(crate) fn output_path(path: &Path) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("output must name a file"))?;
    Ok(fs::canonicalize(parent)?.join(name))
}

pub(crate) fn aliases(left: &Path, right: &Path) -> bool {
    paths_alias(
        left,
        fs::metadata(left).ok().as_ref(),
        right,
        fs::metadata(right).ok().as_ref(),
    )
}

fn paths_alias(
    left: &Path,
    left_meta: Option<&fs::Metadata>,
    right: &Path,
    right_meta: Option<&fs::Metadata>,
) -> bool {
    if left == right {
        return true;
    }
    #[cfg(unix)]
    if let (Some(left), Some(right)) = (left_meta, right_meta) {
        return left.dev() == right.dev() && left.ino() == right.ino();
    }
    #[cfg(not(unix))]
    let _ = (left_meta, right_meta);
    false
}

fn target_metadata(path: &Path) -> Result<Option<fs::Metadata>, OutputError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(OutputError::EditLayout(format!(
            "target {} is a symbolic link",
            path.display()
        ))),
        Ok(metadata) => Ok(Some(metadata)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(OutputError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_optional_target(path: &Path) -> Result<Option<Vec<u8>>, OutputError> {
    if target_metadata(path)?.is_none() {
        return Ok(None);
    }
    read_target(path).map(Some)
}

fn access_permissions(path: &Path) -> Result<fs::Permissions, OutputError> {
    let permissions = fs::metadata(path)
        .map_err(|source| OutputError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    #[cfg(unix)]
    let permissions = fs::Permissions::from_mode(permissions.mode() & 0o777);
    Ok(permissions)
}

fn read_target(path: &Path) -> Result<Vec<u8>, OutputError> {
    fs::read(path).map_err(|source| OutputError::Read {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn write_diff(
    writer: &mut impl Write,
    path: &Path,
    existing: Option<&[u8]>,
    contents: &str,
) -> Result<(), OutputError> {
    writer
        .write_all(diff_text(path, existing, contents)?.as_bytes())
        .map_err(OutputError::Stdout)
}

pub(crate) fn diff_text(
    path: &Path,
    existing: Option<&[u8]>,
    contents: &str,
) -> Result<String, OutputError> {
    let name = path
        .to_str()
        .filter(|name| !name.contains(['\t', '\r', '\n']))
        .ok_or_else(|| OutputError::DiffPath(path.to_path_buf()))?;
    // A tab terminates the filename field, including filenames containing spaces.
    let from = if existing.is_some() {
        format!("{name}\t")
    } else {
        "/dev/null".to_owned()
    };
    let existing = std::str::from_utf8(existing.unwrap_or_default()).map_err(|source| {
        OutputError::DiffEncoding {
            path: path.to_path_buf(),
            source,
        }
    })?;
    Ok(similar::TextDiff::from_lines(existing, contents)
        .unified_diff()
        .header(&from, &format!("{name}\t"))
        .to_string())
}

/// One conflict/write loop for generated files and source-bound edits. The caller
/// supplies its live input guard; every menu and publication attempt rechecks it.
fn publish_one(
    diff: &mut impl Write,
    path: &Path,
    contents: &str,
    existing: Option<&[u8]>,
    permission_source: Option<&Path>,
    choose: &mut impl FnMut(&Path) -> Result<ConflictAction, OutputError>,
    mut check: impl FnMut() -> Result<(), OutputError>,
) -> Result<EditOutcome, OutputError> {
    loop {
        check()?;
        if read_optional_target(path)?.as_deref() != existing {
            return Err(OutputError::Changed(path.to_path_buf()));
        }
        if existing == Some(contents.as_bytes()) {
            return Ok(EditOutcome::Unchanged(path.to_path_buf()));
        }
        let action = if existing.is_none() {
            ConflictAction::Overwrite
        } else {
            choose(path)?
        };
        check()?;
        match action {
            ConflictAction::Skip => return Ok(EditOutcome::Skipped(path.to_path_buf())),
            ConflictAction::Copy => {
                let permissions = permission_source.map(access_permissions).transpose()?;
                let mut number = 0;
                loop {
                    check()?;
                    let copy = copy_path(path, number);
                    match publish_with_permissions(
                        &copy,
                        contents.as_bytes(),
                        false,
                        permissions.clone(),
                    ) {
                        Ok(()) => return Ok(EditOutcome::Written(copy)),
                        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => number += 1,
                        Err(source) => return Err(OutputError::Write { path: copy, source }),
                    }
                }
            }
            ConflictAction::Diff | ConflictAction::Overwrite => {
                if read_optional_target(path)?.as_deref() != existing {
                    return Err(OutputError::Changed(path.to_path_buf()));
                }
                if matches!(action, ConflictAction::Diff) {
                    write_diff(diff, path, existing, contents)?;
                    continue;
                }
                let permissions = existing
                    .map(|_| path)
                    .or(permission_source)
                    .map(access_permissions)
                    .transpose()?;
                check()?;
                publish_with_permissions(
                    path,
                    contents.as_bytes(),
                    existing.is_some(),
                    permissions,
                )
                .map_err(|source| OutputError::Write {
                    path: path.to_path_buf(),
                    source,
                })?;
                return Ok(EditOutcome::Written(path.to_path_buf()));
            }
        }
    }
}

fn copy_path(path: &Path, number: u64) -> PathBuf {
    if number == 0 {
        path.with_added_extension("new")
    } else {
        path.with_added_extension(format!("new.{number}"))
    }
}

/// Atomically publishes a generated stage artifact, not an arbitrary user file.
/// Callers retain their source guards; this boundary refuses aliases and directories.
pub(crate) fn write_artifact(path: &Path, contents: &[u8]) -> io::Result<()> {
    let permissions = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "artifact {} must be a regular file, not a symlink",
                        path.display()
                    ),
                ));
            }
            #[cfg(unix)]
            if metadata.nlink() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "artifact {} must not have multiple hard links",
                        path.display()
                    ),
                ));
            }
            let permissions = metadata.permissions();
            #[cfg(unix)]
            let permissions = fs::Permissions::from_mode(permissions.mode() & 0o777);
            Some(permissions)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    publish_with_permissions(path, contents, permissions.is_some(), permissions)
}

fn publish_with_permissions(
    path: &Path,
    contents: &[u8],
    replace: bool,
    permissions: Option<fs::Permissions>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(".ruyipack.");
    // Replacement staging stays private until the complete content is ready.
    #[cfg(unix)]
    if !replace && permissions.is_none() {
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut file = builder.tempfile_in(parent)?;
    file.write_all(contents)?;
    if let Some(permissions) = permissions {
        file.as_file().set_permissions(permissions)?;
    }
    file.as_file().sync_all()?;
    let result = if replace {
        file.persist(path)
    } else {
        file.persist_noclobber(path)
    };
    result.map(|_| ()).map_err(|error| error.error)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum OutputError {
    #[error("failed to read {}: {source}", .path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to write {}: {source}", .path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("failed to write output to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
    #[error("{0}")]
    Selection(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("{} changed while awaiting confirmation; run the command again", .0.display())]
    Changed(PathBuf),
    #[error("source {} changed since the candidate was prepared; no further files were written", .0.display())]
    SourceChanged(PathBuf),
    #[error("cannot publish edit batch: {0}")]
    EditLayout(String),
    #[error(
        "batch stopped after writing {written:?}; remaining files were not published: {source}"
    )]
    Partial {
        written: Vec<PathBuf>,
        #[source]
        source: Box<OutputError>,
    },
    #[error("cannot represent {} in a unified diff header; use a UTF-8 path without tabs or line breaks", .0.display())]
    DiffPath(PathBuf),
    #[error("cannot show a text diff for {}: {source}", .path.display())]
    DiffEncoding {
        path: PathBuf,
        source: std::str::Utf8Error,
    },
}

#[cfg(test)]
mod tests;
