// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line output options and terminal-only conflict selection.

use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use clap::Args;

use crate::file_output::{self, ConflictAction, OutputError, OutputMode};

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
    #[command(flatten)]
    pub(crate) action: OutputActionOptions,
}

#[derive(Args)]
pub(crate) struct OutputActionOptions {
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

/// Which relocation options the calling command actually accepts.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ConflictHint {
    /// The command has `--output FILE`.
    WithOutputPath,
    /// The command derives the file name from NAME and has no `--output`.
    WithoutOutputPath,
}

impl ConflictHint {
    /// Extra help line, absent when the command has no such option.
    fn help_line(self) -> &'static str {
        match self {
            Self::WithOutputPath => "\n      --output FILE        write to another file",
            Self::WithoutOutputPath => "",
        }
    }
}

impl OutputActionOptions {
    pub(crate) fn publish(
        &self,
        path: &Path,
        contents: &str,
        hint: ConflictHint,
    ) -> Result<(), OutputError> {
        let mode = if self.stdout {
            OutputMode::Stdout
        } else if self.diff {
            OutputMode::Diff
        } else {
            OutputMode::Write
        };
        file_output::run(path, contents, mode, |path| {
            if self.force {
                Ok(ConflictAction::Overwrite)
            } else if self.skip_existing {
                Ok(ConflictAction::Skip)
            } else {
                select_action(path, hint)
            }
        })
    }
}

/// Keeps prompts off redirected input and machine-readable stdout.
fn select_action(path: &Path, hint: ConflictHint) -> Result<ConflictAction, OutputError> {
    let conflict = || SelectionError::Conflict {
        path: path.to_path_buf(),
        hint,
    };
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(conflict().into());
    }
    writeln!(io::stderr().lock(), "warning: {}", conflict()).map_err(OutputError::Stderr)?;
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
        .map_err(SelectionError::Prompt)?;
    selected
        .map(|index| choices[index].0)
        .ok_or_else(|| SelectionError::Cancelled(path.to_path_buf()).into())
}

/// Edit conflicts can only involve a single explicit --output destination.
pub(crate) fn select_edit_action(path: &Path) -> Result<ConflictAction, OutputError> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(SelectionError::EditPrompt.into());
    }
    writeln!(
        io::stderr().lock(),
        "These files will change:\n  {}",
        path.display()
    )
    .map_err(OutputError::Stderr)?;
    let choices = [
        (ConflictAction::Skip, "Keep the current files"),
        (ConflictAction::Diff, "Show the diff"),
        (ConflictAction::Copy, "Write copies"),
        (ConflictAction::Overwrite, "Overwrite the target files"),
    ];
    let selected = dialoguer::Select::new()
        .with_prompt("Choose an action for 1 file(s)")
        .items(choices.iter().map(|(_, label)| label))
        .default(0)
        .report(false)
        .interact_opt()
        .map_err(SelectionError::Prompt)?;
    selected
        .map(|index| choices[index].0)
        .ok_or_else(|| SelectionError::Cancelled(path.to_path_buf()).into())
}

#[derive(Debug, thiserror::Error)]
enum SelectionError {
    #[error("{} already exists with different content\nhelp: --force              overwrite the file\n      --diff               show the differences{}\n      --skip-existing      keep the current file\n      --stdout             preview the complete candidate", .path.display(), .hint.help_line())]
    Conflict { path: PathBuf, hint: ConflictHint },
    #[error("no action selected; kept {}", .0.display())]
    Cancelled(PathBuf),
    #[error(
        "output already exists with different content; confirmation requires a terminal\nhelp: use --force to replace it, --output FILE for another path, or --diff to preview the source edit"
    )]
    EditPrompt,
    #[error("failed to read the conflict selection: {0}")]
    Prompt(#[source] dialoguer::Error),
}

impl From<SelectionError> for OutputError {
    fn from(error: SelectionError) -> Self {
        Self::Selection(Box::new(error))
    }
}
