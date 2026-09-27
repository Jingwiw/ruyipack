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

#[derive(Clone, Copy)]
pub(crate) enum OutputMode {
    Write,
    Diff,
    Stdout,
}

/// Outputs validated text without silently replacing different content.
/// A selection authorizes an action, not stale bytes: revalidation stays here.
pub(crate) fn run(
    path: &Path,
    contents: &str,
    mode: OutputMode,
    mut choose: impl FnMut(&Path) -> Result<ConflictAction, OutputError>,
) -> Result<(), OutputError> {
    if matches!(mode, OutputMode::Stdout) {
        return io::stdout()
            .lock()
            .write_all(contents.as_bytes())
            .map_err(OutputError::Stdout);
    }

    if matches!(mode, OutputMode::Diff) {
        return match fs::read(path) {
            Ok(existing) => show_diff(path, Some(&existing), contents),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                show_diff(path, None, contents)
            }
            Err(source) => Err(OutputError::Read {
                path: path.to_path_buf(),
                source,
            }),
        };
    }

    loop {
        match fs::read(path) {
            Ok(existing) if existing == contents.as_bytes() => return Ok(()),
            Ok(existing) => {
                loop {
                    let selected = choose(path)?;
                    // Every selection still refers to the bytes seen before the first menu.
                    if matches!(selected, ConflictAction::Overwrite | ConflictAction::Diff)
                        && read_target(path)? != existing
                    {
                        return Err(OutputError::Changed(path.to_path_buf()));
                    }
                    if matches!(selected, ConflictAction::Diff) {
                        show_diff(path, Some(&existing), contents)?;
                        continue;
                    }
                    return match selected {
                        ConflictAction::Overwrite => publish(path, contents.as_bytes(), true)
                            .map_err(|source| OutputError::Write {
                                path: path.to_path_buf(),
                                source,
                            }),
                        ConflictAction::Diff => unreachable!("diff returns to the menu"),
                        ConflictAction::Copy => write_copy(path, contents.as_bytes()),
                        ConflictAction::Skip => {
                            writeln!(io::stderr().lock(), "Kept {}", path.display())
                                .map_err(OutputError::Stderr)?;
                            Ok(())
                        }
                    };
                }
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                match publish(path, contents.as_bytes(), false) {
                    Ok(()) => return Ok(()),
                    // A competing creator is handled by the same conflict policy.
                    Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                        // A dangling link is an occupied destination, not a missing file.
                        read_target(path)?;
                    }
                    Err(source) => {
                        return Err(OutputError::Write {
                            path: path.to_path_buf(),
                            source,
                        });
                    }
                }
            }
            Err(source) => {
                return Err(OutputError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }
}

/// One validated candidate and the exact source bytes from which it was prepared.
pub(crate) struct EditFile<'a> {
    pub(crate) source_path: &'a Path,
    pub(crate) original: &'a str,
    pub(crate) contents: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) enum EditMode {
    Write,
    Diff,
    Stdout,
    Overwrite,
}

/// Observed result for one edit destination.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditOutcome {
    Written(PathBuf),
    Unchanged(PathBuf),
    Skipped(PathBuf),
}

impl EditOutcome {
    pub(crate) fn write_human(&self, writer: &mut impl Write) -> io::Result<()> {
        let (action, path) = match self {
            Self::Written(path) => ("Wrote", path),
            Self::Unchanged(path) => ("Unchanged", path),
            Self::Skipped(path) => ("Kept", path),
        };
        writeln!(writer, "{action} {}", path.display())
    }
}

/// Publishes checked candidates, retaining exact paths on partial failure.
pub(crate) fn run_edits(
    files: &[EditFile<'_>],
    output: Option<&Path>,
    mode: EditMode,
    mut choose: impl FnMut(&Path) -> Result<ConflictAction, OutputError>,
) -> Result<Vec<EditOutcome>, OutputError> {
    let mut outcomes = Vec::new();
    match run_edit_batch(files, output, mode, &mut choose, &mut outcomes) {
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
    files: &[EditFile<'_>],
    output: Option<&Path>,
    mode: EditMode,
    choose: &mut impl FnMut(&Path) -> Result<ConflictAction, OutputError>,
    outcomes: &mut Vec<EditOutcome>,
) -> Result<(), OutputError> {
    let mut overwritten = vec![false; files.len()];
    check_sources(files, &overwritten)?;
    match mode {
        EditMode::Stdout => {
            let [file] = files else {
                return Err(OutputError::EditLayout(
                    "--stdout requires exactly one file".into(),
                ));
            };
            return io::stdout()
                .lock()
                .write_all(file.contents.as_bytes())
                .map_err(OutputError::Stdout);
        }
        EditMode::Diff => return show_edit_diffs(files),
        _ => {}
    }
    if files.is_empty() {
        return Ok(());
    }
    let targets = edit_targets(files, output)?;
    let existing = targets
        .iter()
        .map(|path| read_optional_target(path))
        .collect::<Result<Vec<_>, _>>()?;
    if files
        .iter()
        .zip(&targets)
        .all(|(file, target)| target == file.source_path && file.contents == file.original)
    {
        outcomes.extend(targets.into_iter().map(EditOutcome::Unchanged));
        return Ok(());
    }
    // Writing back to the source is the edit action, not an output conflict.
    let no_conflicts =
        files
            .iter()
            .zip(&targets)
            .zip(&existing)
            .all(|((file, target), existing)| {
                target == file.source_path
                    || existing
                        .as_deref()
                        .is_none_or(|bytes| bytes == file.contents.as_bytes())
            });
    let action = match mode {
        _ if no_conflicts => ConflictAction::Overwrite,
        EditMode::Overwrite => ConflictAction::Overwrite,
        EditMode::Write => loop {
            check_sources(files, &overwritten)?;
            // Only a single explicit output can conflict; in-place writes need no menu.
            let selected = choose(&targets[0])?;
            check_sources(files, &overwritten)?;
            if matches!(selected, ConflictAction::Diff) {
                for ((file, target), existing) in files.iter().zip(&targets).zip(&existing) {
                    if read_optional_target(target)? != *existing {
                        return Err(OutputError::Changed(target.clone()));
                    }
                    show_diff(target, existing.as_deref(), file.contents)?;
                }
            } else {
                break selected;
            }
        },
        _ => unreachable!("read-only edit modes already returned"),
    };
    if matches!(action, ConflictAction::Skip) {
        outcomes.extend(targets.into_iter().map(EditOutcome::Skipped));
        return Ok(());
    }
    for (index, (file, target)) in files.iter().zip(&targets).enumerate() {
        check_sources(files, &overwritten)?;
        let path = if matches!(action, ConflictAction::Copy) {
            let permissions = access_permissions(file.source_path)?;
            let mut number = 0_u64;
            loop {
                // An occupied copy name may have appeared after the menu.
                check_sources(files, &overwritten)?;
                let copy = copy_path(target, number);
                match publish_with_permissions(
                    &copy,
                    file.contents.as_bytes(),
                    false,
                    Some(permissions.clone()),
                ) {
                    Ok(()) => break copy,
                    Err(source) if source.kind() == io::ErrorKind::AlreadyExists => number += 1,
                    Err(source) => return Err(OutputError::Write { path: copy, source }),
                }
            }
        } else {
            if read_optional_target(target)? != existing[index] {
                return Err(OutputError::Changed(target.clone()));
            }
            if existing[index].as_deref() == Some(file.contents.as_bytes()) {
                outcomes.push(EditOutcome::Unchanged(target.clone()));
                continue;
            }
            let permissions = access_permissions(if existing[index].is_some() {
                target
            } else {
                file.source_path
            })?;
            // Do not let --force bypass a stale source, including another batch member.
            check_sources(files, &overwritten)?;
            publish_with_permissions(
                target,
                file.contents.as_bytes(),
                existing[index].is_some(),
                Some(permissions),
            )
            .map_err(|source| OutputError::Write {
                path: target.clone(),
                source,
            })?;
            if target == file.source_path {
                overwritten[index] = true;
            }
            target.clone()
        };
        outcomes.push(EditOutcome::Written(path));
    }
    Ok(())
}

fn check_sources(files: &[EditFile<'_>], overwritten: &[bool]) -> Result<(), OutputError> {
    for (file, overwritten) in files.iter().zip(overwritten) {
        if !overwritten
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

fn show_edit_diffs(files: &[EditFile<'_>]) -> Result<(), OutputError> {
    for file in files {
        show_diff(
            file.source_path,
            Some(file.original.as_bytes()),
            file.contents,
        )?;
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
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|source| OutputError::Read {
        path: parent.to_path_buf(),
        source,
    })?;
    let name = output
        .file_name()
        .ok_or_else(|| OutputError::EditLayout("target must name a file".into()))?;
    let path = parent.join(name);
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

fn show_diff(path: &Path, existing: Option<&[u8]>, contents: &str) -> Result<(), OutputError> {
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
    similar::TextDiff::from_lines(existing, contents)
        .unified_diff()
        .header(&from, &format!("{name}\t"))
        .to_writer(io::stdout().lock())
        .map_err(OutputError::Stdout)
}

/// Creates a candidate sidecar without adding another active file extension.
fn write_copy(path: &Path, contents: &[u8]) -> Result<(), OutputError> {
    let mut number = 0_u64;
    loop {
        let copy = copy_path(path, number);
        match publish(&copy, contents, false) {
            Ok(()) => {
                writeln!(io::stderr().lock(), "Wrote {}", copy.display())
                    .map_err(OutputError::Stderr)?;
                return Ok(());
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => number += 1,
            Err(source) => return Err(OutputError::Write { path: copy, source }),
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

/// Publishes staged bytes with explicit overwrite permission.
fn publish(path: &Path, contents: &[u8], replace: bool) -> io::Result<()> {
    let permissions = if replace {
        let permissions = fs::metadata(path)?.permissions();
        // Preserve access permissions without transferring special mode bits to new content.
        #[cfg(unix)]
        let permissions = fs::Permissions::from_mode(permissions.mode() & 0o777);
        Some(permissions)
    } else {
        None
    };
    publish_with_permissions(path, contents, replace, permissions)
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
    #[error("source {} changed since the edit draft was prepared; no further files were written", .0.display())]
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
