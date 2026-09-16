// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! File publication, conflict actions, and terminal selection.

use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use clap::Args;

use crate::utf8_file;

#[derive(Args)]
pub(crate) struct OutputOptions {
    /// Selects the output file, or the comparison target with --diff.
    #[arg(
        short = 'o',
        long = "output",
        value_name = "FILE",
        conflicts_with = "stdout"
    )]
    pub(crate) path: Option<PathBuf>,
    /// Prints the complete candidate without reading or writing the target.
    #[arg(long, conflicts_with_all = ["diff", "force", "skip_existing"])]
    pub(crate) stdout: bool,
    /// Prints a unified diff without writing files, including for a new target.
    ///
    /// Successful comparisons exit with status 0, even when the files differ.
    #[arg(long, conflicts_with_all = ["force", "skip_existing"])]
    pub(crate) diff: bool,
    /// Replaces an existing target with different content.
    #[arg(short, long, conflicts_with = "skip_existing")]
    force: bool,
    /// Skips existing files without prompting.
    #[arg(long)]
    skip_existing: bool,
}

#[derive(Clone, Copy)]
enum ConflictAction {
    Overwrite,
    Diff,
    Copy,
    Skip,
}

/// Outputs validated text without silently replacing different content.
pub(crate) fn run(path: &Path, contents: &str, options: &OutputOptions) -> Result<(), OutputError> {
    if options.stdout {
        return io::stdout()
            .lock()
            .write_all(contents.as_bytes())
            .map_err(OutputError::Stdout);
    }

    if options.diff {
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

    let action = if options.force {
        Some(ConflictAction::Overwrite)
    } else if options.skip_existing {
        Some(ConflictAction::Skip)
    } else {
        None
    };
    loop {
        match fs::read(path) {
            Ok(existing) if existing == contents.as_bytes() => return Ok(()),
            Ok(existing) => {
                loop {
                    let selected = match action {
                        Some(action) => action,
                        None => select_action(path)?,
                    };
                    // Every selection still refers to the bytes seen before the first menu.
                    if action.is_none()
                        && matches!(selected, ConflictAction::Overwrite | ConflictAction::Diff)
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
    pub(crate) target_path: &'a Path,
    pub(crate) contents: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) enum EditMode {
    Write,
    Diff,
    Stdout,
    Overwrite,
}

/// Publishes a checked edit batch. Individual files are staged, not the whole batch.
pub(crate) fn run_edits(files: &[EditFile<'_>], mode: EditMode) -> Result<(), OutputError> {
    let mut written = Vec::new();
    run_edit_batch(files, mode, &mut written).map_err(|source| {
        if written.is_empty() {
            source
        } else {
            OutputError::Partial {
                written: written
                    .iter()
                    .map(|path: &PathBuf| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                source: Box::new(source),
            }
        }
    })
}

fn run_edit_batch(
    files: &[EditFile<'_>],
    mode: EditMode,
    written: &mut Vec<PathBuf>,
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
    let targets = edit_targets(files)?;
    let existing = targets
        .iter()
        .map(|path| read_optional_target(path))
        .collect::<Result<Vec<_>, _>>()?;
    if files
        .iter()
        .zip(&targets)
        .all(|(file, target)| target == file.source_path && file.contents == file.original)
    {
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
            let selected = select_edit_action(files, &targets, &existing)?;
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
        for target in &targets {
            writeln!(io::stderr().lock(), "Kept {}", target.display())
                .map_err(OutputError::Stderr)?;
        }
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
        written.push(path.clone());
        writeln!(io::stderr().lock(), "Wrote {}", path.display()).map_err(OutputError::Stderr)?;
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

fn select_edit_action(
    files: &[EditFile<'_>],
    targets: &[PathBuf],
    existing: &[Option<Vec<u8>>],
) -> Result<ConflictAction, OutputError> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(OutputError::EditPrompt);
    }
    writeln!(io::stderr().lock(), "These files will change:").map_err(OutputError::Stderr)?;
    for ((file, target), existing) in files.iter().zip(targets).zip(existing) {
        if existing
            .as_deref()
            .is_some_and(|bytes| bytes != file.contents.as_bytes())
        {
            writeln!(io::stderr().lock(), "  {}", target.display()).map_err(OutputError::Stderr)?;
        }
    }
    let choices = [
        (ConflictAction::Skip, "Keep the current files"),
        (ConflictAction::Diff, "Show the diff"),
        (ConflictAction::Copy, "Write copies"),
        (ConflictAction::Overwrite, "Overwrite the target files"),
    ];
    let selected = dialoguer::Select::new()
        .with_prompt(format!("Choose an action for {} file(s)", files.len()))
        .items(choices.iter().map(|(_, label)| label))
        .default(0)
        .report(false)
        .interact_opt()
        .map_err(OutputError::Prompt)?;
    selected
        .map(|index| choices[index].0)
        .ok_or_else(|| OutputError::Cancelled(targets[0].clone()))
}

/// Resolves destination parents while keeping the final path entry explicit.
fn edit_targets(files: &[EditFile<'_>]) -> Result<Vec<PathBuf>, OutputError> {
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
    let mut targets: Vec<(PathBuf, Option<fs::Metadata>)> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let parent = file
            .target_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = fs::canonicalize(parent).map_err(|source| OutputError::Read {
            path: parent.to_path_buf(),
            source,
        })?;
        let name = file
            .target_path
            .file_name()
            .ok_or_else(|| OutputError::EditLayout("target must name a file".into()))?;
        let path = parent.join(name);
        let metadata = target_metadata(&path)?;
        for (source_index, (source, data)) in sources.iter().enumerate() {
            if paths_alias(&path, metadata.as_ref(), source, Some(data))
                && (index != source_index || path != *source)
            {
                return Err(OutputError::EditLayout(format!(
                    "target {} aliases source {}",
                    path.display(),
                    source.display()
                )));
            }
        }
        if targets
            .iter()
            .any(|(other, data)| paths_alias(&path, metadata.as_ref(), other, data.as_ref()))
        {
            return Err(OutputError::EditLayout(format!(
                "duplicate or aliased target {}",
                path.display()
            )));
        }
        targets.push((path, metadata));
    }
    Ok(targets.into_iter().map(|(path, _)| path).collect())
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

/// Keeps prompts off redirected input and machine-readable stdout.
fn select_action(path: &Path) -> Result<ConflictAction, OutputError> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(OutputError::Conflict(path.to_path_buf()));
    }
    writeln!(
        io::stderr().lock(),
        "warning: {}",
        OutputError::Conflict(path.to_path_buf())
    )
    .map_err(OutputError::Stderr)?;
    let choices = [
        (ConflictAction::Skip, "Keep the current file"),
        (ConflictAction::Diff, "Show the diff"),
        (ConflictAction::Copy, "Write a copy"),
        (ConflictAction::Overwrite, "Overwrite the current file"),
    ];
    let selected = dialoguer::Select::new()
        .with_prompt("Choose an action")
        .items(choices.iter().map(|(_, label)| label))
        .default(0)
        .report(false)
        .interact_opt()
        .map_err(OutputError::Prompt)?;
    selected
        .map(|index| choices[index].0)
        .ok_or_else(|| OutputError::Cancelled(path.to_path_buf()))
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
    let mut name = path.as_os_str().to_owned();
    name.push(".new");
    if number != 0 {
        name.push(format!(".{number}"));
    }
    PathBuf::from(name)
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
    #[error("{} already exists with different content\nhelp: --force              overwrite the file\n      --diff               show the differences\n      --output FILE        write to another file\n      --skip-existing      keep the current file\n      --stdout             preview the complete candidate", .0.display())]
    Conflict(PathBuf),
    #[error("no action selected; kept {}", .0.display())]
    Cancelled(PathBuf),
    #[error("{} changed while awaiting confirmation; run the command again", .0.display())]
    Changed(PathBuf),
    #[error("source {} changed since the edit draft was prepared; no further files were written", .0.display())]
    SourceChanged(PathBuf),
    #[error("cannot publish edit batch: {0}")]
    EditLayout(String),
    #[error(
        "output already exists with different content; confirmation requires a terminal\nhelp: use --force to replace it, --output FILE for another path, or --diff to preview the source edit"
    )]
    EditPrompt,
    #[error("batch stopped after writing {written}; remaining files were not published: {source}")]
    Partial {
        written: String,
        #[source]
        source: Box<OutputError>,
    },
    #[error("failed to read the conflict selection: {0}")]
    Prompt(#[source] dialoguer::Error),
    #[error("cannot represent {} in a unified diff header; use a UTF-8 path without tabs or line breaks", .0.display())]
    DiffPath(PathBuf),
    #[error("cannot show a text diff for {}: {source}", .path.display())]
    DiffEncoding {
        path: PathBuf,
        source: std::str::Utf8Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(directory: &Path, name: &str, contents: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, contents).unwrap();
        path.canonicalize().unwrap()
    }

    #[test]
    fn all_sources_are_checked_before_force_writes_any_member() {
        let directory = tempfile::tempdir().unwrap();
        let first = source(directory.path(), "first.spec", "first\n");
        let second = source(directory.path(), "second.spec", "changed externally\n");
        let files = [
            EditFile {
                source_path: &first,
                original: "first\n",
                target_path: &first,
                contents: "edited first\n",
            },
            EditFile {
                source_path: &second,
                original: "second\n",
                target_path: &second,
                contents: "edited second\n",
            },
        ];
        assert!(
            matches!(run_edits(&files, EditMode::Overwrite), Err(OutputError::SourceChanged(path)) if path == second)
        );
        assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "changed externally\n");
    }

    #[cfg(unix)]
    #[test]
    fn repeated_source_checks_reject_a_same_bytes_symlink_replacement() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let input = source(directory.path(), "input.spec", "original\n");
        let replacement = source(directory.path(), "replacement.spec", "original\n");
        let target = directory.path().join("output.spec");
        let files = [EditFile {
            source_path: &input,
            original: "original\n",
            target_path: &target,
            contents: "edited\n",
        }];
        check_sources(&files, &[false]).unwrap();
        fs::remove_file(&input).unwrap();
        symlink(&replacement, &input).unwrap();
        assert!(matches!(
            check_sources(&files, &[false]),
            Err(OutputError::SourceChanged(path)) if path == input
        ));
    }

    #[test]
    fn successful_overwrites_do_not_invalidate_the_remaining_batch() {
        let directory = tempfile::tempdir().unwrap();
        let first = source(directory.path(), "first.spec", "first\n");
        let second = source(directory.path(), "second.spec", "second\n");
        let files = [
            EditFile {
                source_path: &first,
                original: "first\n",
                target_path: &first,
                contents: "edited first\n",
            },
            EditFile {
                source_path: &second,
                original: "second\n",
                target_path: &second,
                contents: "edited second\n",
            },
        ];
        run_edits(&files, EditMode::Overwrite).unwrap();
        assert_eq!(fs::read_to_string(first).unwrap(), "edited first\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "edited second\n");
    }

    #[test]
    fn cross_source_targets_are_rejected_before_publication() {
        let directory = tempfile::tempdir().unwrap();
        let first = source(directory.path(), "first.spec", "first\n");
        let second = source(directory.path(), "second.spec", "second\n");
        let files = [
            EditFile {
                source_path: &first,
                original: "first\n",
                target_path: &second,
                contents: "edited first\n",
            },
            EditFile {
                source_path: &second,
                original: "second\n",
                target_path: &first,
                contents: "edited second\n",
            },
        ];
        assert!(matches!(
            run_edits(&files, EditMode::Overwrite),
            Err(OutputError::EditLayout(_))
        ));
        assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
        assert_eq!(fs::read_to_string(second).unwrap(), "second\n");
    }

    #[cfg(unix)]
    #[test]
    fn hardlinked_sources_and_targets_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let first = source(directory.path(), "first.spec", "first\n");
        let alias = directory.path().join("alias.spec");
        fs::hard_link(&first, &alias).unwrap();
        let alias = alias.canonicalize().unwrap();
        let own_alias = [EditFile {
            source_path: &first,
            original: "first\n",
            target_path: &alias,
            contents: "edited\n",
        }];
        assert!(matches!(
            run_edits(&own_alias, EditMode::Overwrite),
            Err(OutputError::EditLayout(_))
        ));
        let duplicate_sources = [
            EditFile {
                source_path: &first,
                original: "first\n",
                target_path: &first,
                contents: "edited\n",
            },
            EditFile {
                source_path: &alias,
                original: "first\n",
                target_path: &alias,
                contents: "edited\n",
            },
        ];
        assert!(matches!(
            run_edits(&duplicate_sources, EditMode::Overwrite),
            Err(OutputError::EditLayout(_))
        ));
        assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
    }

    #[cfg(unix)]
    #[test]
    fn edits_preserve_access_permissions_on_existing_and_new_targets() {
        let directory = tempfile::tempdir().unwrap();
        let input = source(directory.path(), "input.spec", "original\n");
        fs::set_permissions(&input, fs::Permissions::from_mode(0o640)).unwrap();
        let target = directory.path().canonicalize().unwrap().join("new.spec");
        run_edits(
            &[EditFile {
                source_path: &input,
                original: "original\n",
                target_path: &target,
                contents: "edited\n",
            }],
            EditMode::Overwrite,
        )
        .unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        run_edits(
            &[EditFile {
                source_path: &input,
                original: "original\n",
                target_path: &input,
                contents: "edited\n",
            }],
            EditMode::Overwrite,
        )
        .unwrap();
        assert_eq!(
            fs::metadata(&input).unwrap().permissions().mode() & 0o7777,
            0o640
        );
    }
}
