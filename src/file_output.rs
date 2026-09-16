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
use std::os::unix::fs::PermissionsExt;

use clap::Args;

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
                let selected = match action {
                    Some(action) => action,
                    None => select_action(path)?,
                };
                // Confirmation and diffs refer to the contents seen before opening the menu.
                if action.is_none()
                    && matches!(selected, ConflictAction::Overwrite | ConflictAction::Diff)
                    && read_target(path)? != existing
                {
                    return Err(OutputError::Changed(path.to_path_buf()));
                }
                return match selected {
                    ConflictAction::Overwrite => {
                        publish(path, contents.as_bytes(), true).map_err(|source| {
                            OutputError::Write {
                                path: path.to_path_buf(),
                                source,
                            }
                        })
                    }
                    ConflictAction::Diff => show_diff(path, Some(&existing), contents),
                    ConflictAction::Copy => write_copy(path, contents.as_bytes()),
                    ConflictAction::Skip => {
                        writeln!(io::stderr().lock(), "Kept {}", path.display())
                            .map_err(OutputError::Stderr)?;
                        Ok(())
                    }
                };
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

/// Prints a read-only comparison against the captured SPEC source.
pub(crate) fn show_edit_diff(
    path: &Path,
    original: &str,
    contents: &str,
) -> Result<(), OutputError> {
    show_diff(path, Some(original.as_bytes()), contents)
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
        let mut name = path.as_os_str().to_owned();
        name.push(".new");
        if number != 0 {
            name.push(format!(".{number}"));
        }
        let copy = PathBuf::from(name);
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
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(".ruyipack.");
    // Replacement staging stays private until the complete content is ready.
    #[cfg(unix)]
    if !replace {
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
