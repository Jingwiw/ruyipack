// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line presentation and terminal-only conflict selection.

use std::{
    fmt,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use clap::{Args, ValueEnum};

use crate::file_output::{self, ConflictAction, OutputError, OutputMode};

/// Human diagnostics only: structured reports retain their original paths and levels.
#[derive(Clone, Copy)]
pub(crate) enum HumanLevel {
    Info,
    Warn,
    Error,
}

/// Writes one stderr diagnostic while retaining the caller's I/O error contract.
/// The supplied subject may be a WORK name; actual paths are displayed relative to cwd.
pub(crate) fn human(
    writer: &mut impl Write,
    level: HumanLevel,
    subject: Option<&Path>,
    message: fmt::Arguments<'_>,
) -> io::Result<()> {
    let color = color_allowed(
        io::stderr().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
        std::env::var_os("TERM").as_deref() == Some(std::ffi::OsStr::new("dumb")),
    );
    human_with_color(writer, level, subject, message, color)
}

/// Diagnostic coordinates refer to the SPEC identified by the surrounding report.
/// Zero means the producer supplied no coordinate, not a synthetic line zero.
pub(crate) fn diagnostic(
    writer: &mut impl Write,
    level: HumanLevel,
    start: Option<(u32, u32)>,
    code: Option<&str>,
    message: fmt::Arguments<'_>,
) -> io::Result<()> {
    let location = match start {
        Some((line, column)) if line > 0 && column > 0 => format!("[{line}:{column}]"),
        Some((line, _)) if line > 0 => format!("[{line}]"),
        _ => String::new(),
    };
    let code = code.map_or_else(String::new, |code| format!(" [{code}]"));
    human(
        writer,
        level,
        None,
        format_args!("spec{location}{code}: {message}"),
    )
}

fn color_allowed(terminal: bool, no_color: bool, dumb: bool) -> bool {
    terminal && !no_color && !dumb
}

fn human_with_color(
    writer: &mut impl Write,
    level: HumanLevel,
    subject: Option<&Path>,
    message: fmt::Arguments<'_>,
    color: bool,
) -> io::Result<()> {
    use dialoguer::console::Style;

    let (prefix, style) = match level {
        HumanLevel::Info => ("[INFO]", Style::new().color256(8).dim()),
        HumanLevel::Warn => ("[WARN]", Style::new().yellow()),
        HumanLevel::Error => ("[ERROR]", Style::new().red()),
    };
    // Explicitly guard NO_COLOR and dumb/non-terminal streams: console's force
    // environment can otherwise override the terminal check.
    write!(writer, "{} ", style.force_styling(color).apply_to(prefix))?;
    if let Some(subject) = subject {
        write!(writer, "{}: ", human_path(subject).display())?;
    }
    writeln!(writer, "{message}")
}

/// Presentation only; never use this for diff headers, resumable commands or receipts.
/// Keeping the relative path (rather than the basename) distinguishes batch members.
pub(crate) fn human_path(path: &Path) -> PathBuf {
    if !path.is_absolute() {
        return path.to_path_buf();
    }
    let Ok(current) = std::env::current_dir() else {
        return path.to_path_buf();
    };
    // cwd is resolved by the OS (e.g. /private/var on macOS). Normalize only
    // existing parents so the displayed file entry and nonexistent suffix stay
    // intact, including a diagnostic's " (candidate)" suffix.
    let normalized = path.parent().and_then(|parent| {
        parent.ancestors().find_map(|ancestor| {
            ancestor.canonicalize().ok().map(|canonical| {
                canonical.join(path.strip_prefix(ancestor).expect("path ancestor"))
            })
        })
    });
    relative_path(normalized.as_deref().unwrap_or(path), &current)
}

fn relative_path(path: &Path, current: &Path) -> PathBuf {
    if !path.is_absolute() {
        return path.to_path_buf();
    }
    let path_parts: Vec<_> = path.components().collect();
    let current_parts: Vec<_> = current.components().collect();
    let shared = path_parts
        .iter()
        .zip(&current_parts)
        .take_while(|(left, right)| left == right)
        .count();
    // Different Windows drives cannot be represented as a relative path.
    if shared == 0 {
        return path.to_path_buf();
    }
    let mut relative = PathBuf::new();
    for _ in &current_parts[shared..] {
        relative.push("..");
    }
    for component in &path_parts[shared..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    relative
}

/// Presentation choice shared by check, inspect, generation and edit reports.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum ReportFormat {
    Human,
    Toml,
}

/// File input and output failures shared by the read-only report commands.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportError {
    #[error("{0}")]
    Workspace(#[from] io::Error),
    #[error("{0}")]
    Projection(String),
    #[error("{0}")]
    Manifest(#[from] crate::render::RenderError),
    #[error("{0}")]
    Input(#[from] crate::utf8_file::Utf8FileError),
    #[error("failed to write output to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
}

/// A read failure is a business result in TOML mode, not an absent report.
/// The input producer owns file reading or WORK resolution, including its lock.
pub(crate) fn report_input<T>(
    result: Result<T, impl Into<ReportError>>,
    input: &str,
    format: ReportFormat,
) -> Result<Option<T>, ReportError> {
    match result.map_err(Into::into) {
        Ok(input) => Ok(Some(input)),
        Err(error) if matches!(format, ReportFormat::Toml) => {
            write_failure(
                crate::report::Input {
                    display_path: input.into(),
                    sha256: None,
                    revision: None,
                },
                crate::report::failure("input-read", error),
            )?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn write_failure(
    input: crate::report::Input<'_>,
    error: crate::report::Failure,
) -> Result<(), ReportError> {
    #[derive(serde::Serialize)]
    struct FailureReport<'a> {
        format_version: u32,
        valid: bool,
        tool: crate::tool::Identity,
        input: crate::report::Input<'a>,
        error: crate::report::Failure,
    }
    crate::report::write(
        &mut io::stdout().lock(),
        &FailureReport {
            format_version: 2,
            valid: false,
            tool: crate::tool::identity(),
            input,
            error,
        },
    )
    .map_err(ReportError::Stdout)
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

impl OutputActionOptions {
    pub(crate) fn emit(&self, path: &Path, contents: &str) -> Result<(), OutputError> {
        let mode = if self.stdout {
            OutputMode::Stdout
        } else if self.diff {
            OutputMode::Diff
        } else {
            OutputMode::Write
        };
        file_output::run(path, contents, mode, |path| self.choose(path))
    }

    /// Generation has an input file too: reuse publication's conflict-time source checks.
    pub(crate) fn emit_from(
        &self,
        path: &Path,
        contents: &str,
        source_path: &Path,
        original: &str,
    ) -> Result<Vec<file_output::EditOutcome>, OutputError> {
        if self.stdout || self.diff {
            return self.emit(path, contents).map(|()| Vec::new());
        }
        let file = file_output::EditFile {
            source_path,
            original,
            contents,
        };
        file_output::run_edits(&[file], Some(path), |path| self.choose(path)).and_then(|outcomes| {
            for outcome in &outcomes {
                if matches!(outcome, file_output::EditOutcome::Skipped(_)) {
                    outcome
                        .write_human(&mut io::stderr().lock())
                        .map_err(OutputError::Stderr)?;
                }
            }
            Ok(outcomes)
        })
    }

    fn choose(&self, path: &Path) -> Result<ConflictAction, OutputError> {
        if self.force {
            Ok(ConflictAction::Overwrite)
        } else if self.skip_existing {
            Ok(ConflictAction::Skip)
        } else {
            select_action(path)
        }
    }
}

/// Keeps prompts off redirected input and machine-readable stdout.
fn select_action(path: &Path) -> Result<ConflictAction, OutputError> {
    let conflict = || SelectionError::Conflict {
        path: path.to_path_buf(),
    };
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(conflict().into());
    }
    human(
        &mut io::stderr().lock(),
        HumanLevel::Warn,
        Some(path),
        format_args!("already exists with different content\nhelp: --force              overwrite the file\n      --diff               show the differences\n      --skip-existing      keep the current file\n      --stdout             preview the complete candidate"),
    )
    .map_err(OutputError::Stderr)?;
    choose_conflict_action(path)
}

/// Edit conflicts can only involve a single explicit --output destination.
pub(crate) fn select_edit_action(path: &Path) -> Result<ConflictAction, OutputError> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(SelectionError::EditPrompt.into());
    }
    human(
        &mut io::stderr().lock(),
        HumanLevel::Info,
        None,
        format_args!("This file will change:\n  {}", human_path(path).display()),
    )
    .map_err(OutputError::Stderr)?;
    choose_conflict_action(path)
}

/// Choose an editing scope, not an implicit full-SPEC conversion. Scripts must
/// supply their scope explicitly; they never receive terminal menu output.
pub(crate) fn select_edit_field() -> Result<Option<&'static str>, String> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Ok(None);
    }
    let choices = [
        ("package.version", "Version"),
        (
            "sources.0",
            "Source0 URL and SHA-256 (other numbers: --field sources.N)",
        ),
        ("build-requires.rpm", "Build dependencies"),
        ("package.summary", "Summary"),
        ("package.license", "License"),
        ("package.files", "File lists"),
    ];
    dialoguer::Select::new()
        .with_prompt("What do you want to edit? (other fields: --field FIELD)")
        .items(choices.iter().map(|(_, label)| label))
        .default(0)
        .report(false)
        .interact_opt()
        .map_err(|e| e.to_string())?
        .map(|index| Some(choices[index].0))
        .ok_or_else(|| "edit cancelled; no files changed".into())
}

fn choose_conflict_action(path: &Path) -> Result<ConflictAction, OutputError> {
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

#[derive(Debug, thiserror::Error)]
enum SelectionError {
    #[error("{} already exists with different content\nhelp: --force              overwrite the file\n      --diff               show the differences\n      --skip-existing      keep the current file\n      --stdout             preview the complete candidate", .path.display())]
    Conflict { path: PathBuf },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_failures_are_complete_toml_and_encoding_errors_write_nothing() {
        let mut output = Vec::new();
        crate::report::write(
            &mut output,
            &crate::report::failure("input-read", "路径不存在\n\"input.spec\""),
        )
        .unwrap();
        assert_eq!(output.last(), Some(&b'\n'));
        let report: toml::Table = toml::from_str(std::str::from_utf8(&output).unwrap()).unwrap();
        assert_eq!(report["code"].as_str(), Some("input-read"));
        assert_eq!(
            report["message"].as_str(),
            Some("路径不存在\n\"input.spec\"")
        );

        let mut output = Vec::new();
        assert!(crate::report::write(&mut output, &None::<bool>).is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn human_paths_preserve_distinct_same_named_batch_inputs() {
        let current = std::env::current_dir().unwrap();
        for name in ["first/pkg.spec", "second/pkg.spec"] {
            assert_eq!(human_path(&current.join(name)), Path::new(name));
        }
        assert_eq!(human_path(Path::new("WORK")), Path::new("WORK"));
        assert_eq!(
            relative_path(Path::new("/repo/pkg.spec"), Path::new("/repo/work")),
            Path::new("../pkg.spec")
        );
        assert_eq!(
            relative_path(Path::new("/repo/work"), Path::new("/repo/work")),
            Path::new(".")
        );
    }

    #[cfg(unix)]
    #[test]
    fn human_paths_resolve_parent_aliases_without_resolving_the_file_entry() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let actual = directory.path().join("actual");
        std::fs::create_dir(&actual).unwrap();
        let alias = directory.path().join("alias");
        symlink(&actual, &alias).unwrap();
        std::fs::write(actual.join("target"), "input").unwrap();
        symlink("target", actual.join("pkg.spec")).unwrap();
        assert_eq!(
            human_path(&alias.join("pkg.spec")),
            human_path(&actual.join("pkg.spec"))
        );
        assert_ne!(
            human_path(&alias.join("pkg.spec")),
            human_path(&actual.join("target"))
        );
        assert_eq!(
            human_path(&alias.join("pkg.spec (candidate)")),
            human_path(&actual.join("pkg.spec (candidate)"))
        );
    }

    #[test]
    fn human_color_is_prefix_only_and_disabled_without_a_suitable_terminal() {
        for (level, color_prefix) in [
            (HumanLevel::Info, "\u{1b}[38;5;8m\u{1b}[2m"),
            (HumanLevel::Warn, "\u{1b}[33m"),
            (HumanLevel::Error, "\u{1b}[31m"),
        ] {
            let mut output = Vec::new();
            human_with_color(
                &mut output,
                level,
                Some(Path::new("WORK")),
                format_args!("same plain body"),
                true,
            )
            .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(output.starts_with(color_prefix), "{output:?}");
            let (prefix, body) = output.split_once(' ').unwrap();
            assert!(prefix.contains('\u{1b}'));
            assert!(prefix.ends_with("\u{1b}[0m"), "{output:?}");
            assert_eq!(body, "WORK: same plain body\n");

            let mut output = Vec::new();
            human_with_color(
                &mut output,
                level,
                Some(Path::new("WORK")),
                format_args!("same plain body"),
                false,
            )
            .unwrap();
            assert!(!output.contains(&0x1b));
        }
        assert!(color_allowed(true, false, false));
        assert!(!color_allowed(false, false, false));
        assert!(!color_allowed(true, true, false));
        assert!(!color_allowed(true, false, true));
    }

    #[test]
    fn diagnostic_positions_preserve_known_coordinates_without_inventing_unknown_ones() {
        for (start, suffix) in [
            (Some((17, 2)), "[17:2]"),
            (Some((17, 0)), "[17]"),
            (Some((0, 0)), ""),
            (None, ""),
        ] {
            let mut output = Vec::new();
            diagnostic(
                &mut output,
                HumanLevel::Warn,
                start,
                Some("RPK005"),
                format_args!("no sha256"),
            )
            .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert_eq!(
                dialoguer::console::strip_ansi_codes(&output),
                format!("[WARN] spec{suffix} [RPK005]: no sha256\n")
            );
        }
    }

    #[test]
    fn human_preserves_writer_errors() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let error = human(&mut Broken, HumanLevel::Warn, None, format_args!("test")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
