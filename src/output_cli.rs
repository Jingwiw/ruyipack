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

use crate::file_output::{self, ConflictAction, EditOutcome, OutputError};

/// Human diagnostics only: structured reports retain their original paths and levels.
#[derive(Clone, Copy)]
pub(crate) enum HumanLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// A diagnostic destination owns its color policy; a buffer never inherits stderr's TTY.
pub(crate) struct HumanOutput<W> {
    writer: W,
    color: bool,
}

static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub(crate) fn set_debug(enabled: bool) {
    let _ = DEBUG.set(enabled);
}

pub(crate) fn debug_enabled() -> bool {
    DEBUG.get().copied().unwrap_or(false)
}

pub(crate) fn stderr() -> HumanOutput<io::StderrLock<'static>> {
    let writer = io::stderr().lock();
    let color = color_allowed(
        writer.is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
        std::env::var_os("TERM").as_deref() == Some(std::ffi::OsStr::new("dumb")),
    );
    HumanOutput::new(writer, color)
}

impl<W: Write> HumanOutput<W> {
    pub(crate) fn new(writer: W, color: bool) -> Self {
        Self { writer, color }
    }

    /// The subject may be a WORK name; actual paths are displayed relative to cwd.
    pub(crate) fn message(
        &mut self,
        level: HumanLevel,
        subject: Option<&Path>,
        message: fmt::Arguments<'_>,
    ) -> io::Result<()> {
        use console::Style;

        if matches!(level, HumanLevel::Debug) && !debug_enabled() {
            return Ok(());
        }

        let (prefix, style) = match level {
            HumanLevel::Debug => ("[DEBUG]", Style::new().dim()),
            HumanLevel::Info => ("[INFO]", Style::new().color256(8).dim()),
            HumanLevel::Warn => ("[WARN]", Style::new().yellow()),
            HumanLevel::Error => ("[ERROR]", Style::new().red()),
        };
        // Force the destination's policy, including when CLICOLOR_FORCE is set.
        write!(
            self.writer,
            "{} ",
            style.force_styling(self.color).apply_to(prefix)
        )?;
        if let Some(subject) = subject {
            write!(self.writer, "{}: ", human_path(subject).display())?;
        }
        writeln!(self.writer, "{message}")
    }

    /// Coordinates refer to the SPEC identified by the surrounding report.
    /// Zero means the producer supplied no coordinate, not a synthetic line zero.
    pub(crate) fn diagnostic(
        &mut self,
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
        self.message(level, None, format_args!("spec{location}{code}: {message}"))
    }

    pub(crate) fn note(&mut self, note: &str) -> io::Result<()> {
        writeln!(self.writer, "  note: {note}")
    }
}

fn color_allowed(terminal: bool, no_color: bool, dumb: bool) -> bool {
    terminal && !no_color && !dumb
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

/// Display an observed publication result; the publisher owns paths and outcomes.
pub(crate) fn write_outcome(
    writer: &mut HumanOutput<impl Write>,
    outcome: &EditOutcome,
) -> io::Result<()> {
    let (action, path) = match outcome {
        EditOutcome::Written(path) => ("Wrote", path),
        EditOutcome::Unchanged(path) => ("Unchanged", path),
        EditOutcome::Skipped(path) => ("Kept", path),
    };
    writer.message(
        HumanLevel::Info,
        None,
        format_args!("{action} {}", human_path(path).display()),
    )
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
    crate::report::write(
        &mut io::stdout().lock(),
        &crate::report::failed(input, error),
    )
    .map_err(ReportError::Stdout)
}

#[derive(Args)]
pub(crate) struct ConflictOptions {
    /// Replaces an existing target with different content.
    #[arg(short, long, conflicts_with = "skip_existing")]
    force: bool,
    /// Skips existing files without prompting.
    #[arg(long)]
    skip_existing: bool,
}

impl ConflictOptions {
    pub(crate) fn publish(&self, path: &Path, contents: &str) -> Result<EditOutcome, OutputError> {
        let outcome = file_output::publish(&mut io::stdout().lock(), path, contents, |path| {
            self.choose(path, ReportFormat::Human)
        })?;
        if matches!(outcome, EditOutcome::Skipped(_))
            || matches!(&outcome, EditOutcome::Written(copy) if copy != path)
        {
            write_outcome(&mut stderr(), &outcome).map_err(OutputError::Stderr)?;
        }
        Ok(outcome)
    }

    /// Generation has an input file too: reuse publication's conflict-time source checks.
    pub(crate) fn publish_from(
        &self,
        path: &Path,
        contents: &str,
        source_path: &Path,
        original: &str,
        format: ReportFormat,
    ) -> Result<Vec<file_output::EditOutcome>, OutputError> {
        let file = file_output::EditFile {
            source_path,
            original,
            contents,
        };
        file_output::run_edits(&mut io::stdout().lock(), &[file], Some(path), |path| {
            self.choose(path, format)
        })
        .and_then(|outcomes| {
            if matches!(format, ReportFormat::Human) {
                for outcome in &outcomes {
                    if matches!(outcome, file_output::EditOutcome::Skipped(_)) {
                        write_outcome(&mut stderr(), outcome).map_err(OutputError::Stderr)?;
                    }
                }
            }
            Ok(outcomes)
        })
    }

    fn choose(&self, path: &Path, format: ReportFormat) -> Result<ConflictAction, OutputError> {
        if self.force {
            Ok(ConflictAction::Overwrite)
        } else if self.skip_existing {
            Ok(ConflictAction::Skip)
        } else {
            select_action(path, format)
        }
    }
}

/// Keeps prompts off redirected input and machine-readable stdout.
fn select_action(path: &Path, format: ReportFormat) -> Result<ConflictAction, OutputError> {
    let conflict = || SelectionError::Conflict {
        path: path.to_path_buf(),
    };
    if matches!(format, ReportFormat::Toml)
        || !io::stdin().is_terminal()
        || !io::stderr().is_terminal()
    {
        return Err(conflict().into());
    }
    stderr().message(
        HumanLevel::Warn,
        Some(path),
        format_args!("already exists with different content\nhelp: --force              overwrite the file\n      --diff               show the differences\n      --skip-existing      keep the current file\n      --stdout             preview the complete candidate"),
    )
    .map_err(OutputError::Stderr)?;
    choose_conflict_action(path)
}

/// Edit conflicts can only involve a single explicit --output destination.
pub(crate) fn select_edit_action(
    path: &Path,
    format: ReportFormat,
) -> Result<ConflictAction, OutputError> {
    if matches!(format, ReportFormat::Toml)
        || !io::stdin().is_terminal()
        || !io::stderr().is_terminal()
    {
        return Err(SelectionError::EditPrompt.into());
    }
    stderr()
        .message(
            HumanLevel::Info,
            None,
            format_args!("This file will change:\n  {}", human_path(path).display()),
        )
        .map_err(OutputError::Stderr)?;
    choose_conflict_action(path)
}

/// Choose an editing scope, not an implicit full-SPEC conversion. Scripts must
/// supply their scope explicitly; they never receive terminal menu output.
pub(crate) fn select_edit_field(
    snapshot: &crate::spec::document::Snapshot<'_>,
) -> Result<Option<String>, String> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Ok(None);
    }
    let mut table = snapshot.document();
    let mut path = String::new();
    loop {
        let mut choices = table.keys().map(String::as_str).collect::<Vec<_>>();
        if path.is_empty()
            && crate::spec::document::table::lookup(snapshot.document(), "build.stages.conf")
                .is_some()
        {
            choices.push("conf");
        }
        let selected = crate::prompt::choose(
            if path.is_empty() {
                "What do you want to edit?"
            } else {
                &path
            },
            &choices,
            None,
        )
        .map_err(|error| error.to_string())?;
        if path.is_empty() && selected == "conf" {
            let mode = crate::prompt::choose("conf", &["-p", "-a", "replace"], None)
                .map_err(|error| error.to_string())?;
            let mode = match mode.as_str() {
                "-p" => "prepend",
                "-a" => "append",
                _ => "replace",
            };
            return Ok(Some(format!("build.stages.conf.{mode}")));
        }
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(&selected);
        if path == "build-requires" {
            let scopes = snapshot.dependency_scopes();
            let scope = if scopes.len() == 1 {
                scopes[0].0
            } else {
                let labels = scopes
                    .iter()
                    .map(|(key, label)| format!("{key}: {label}"))
                    .collect::<Vec<_>>();
                let choices = labels.iter().map(String::as_str).collect::<Vec<_>>();
                let label = crate::prompt::choose(
                    "Declaration scope (conditions are not evaluated)",
                    &choices,
                    None,
                )
                .map_err(|error| error.to_string())?;
                scopes[labels
                    .iter()
                    .position(|text| *text == label)
                    .ok_or("unknown declaration scope")?]
                .0
            };
            let namespace = crate::prompt::choose(
                "BuildRequires namespace",
                crate::dependency::NAMESPACES,
                None,
            )
            .map_err(|error| error.to_string())?;
            return Ok(Some(format!("{scope}.{namespace}")));
        }
        match &table[&selected] {
            toml::Value::Table(next) => table = next,
            _ => return Ok(Some(path)),
        }
    }
}

fn choose_conflict_action(path: &Path) -> Result<ConflictAction, OutputError> {
    let choices = [
        (ConflictAction::Skip, "Keep the current file"),
        (ConflictAction::Diff, "Show the diff"),
        (ConflictAction::Copy, "Write a copy"),
        (ConflictAction::Overwrite, "Overwrite the current file"),
    ];
    let labels = choices.iter().map(|(_, label)| *label).collect::<Vec<_>>();
    let selected =
        crate::prompt::choose("Choose an action", &labels, None).map_err(SelectionError::Prompt)?;
    choices
        .iter()
        .find(|(_, label)| *label == selected)
        .map(|(action, _)| *action)
        .ok_or_else(|| SelectionError::Cancelled(path.to_path_buf()).into())
}

#[derive(Debug, thiserror::Error)]
enum SelectionError {
    #[error("{} already exists with different content\nhelp: --force              overwrite the file\n      --diff               show the differences\n      --skip-existing      keep the current file\n      --stdout             preview the complete candidate", .path.display())]
    Conflict { path: PathBuf },
    #[error("no action selected; kept {}", .0.display())]
    Cancelled(PathBuf),
    #[error(
        "output already exists with different content; select an explicit conflict action or use human mode in a terminal\nhelp: use --force to replace it, --output FILE for another path, or --diff to preview the source edit"
    )]
    EditPrompt,
    #[error("failed to read the conflict selection: {0}")]
    Prompt(#[source] inquire::InquireError),
}

impl From<SelectionError> for OutputError {
    fn from(error: SelectionError) -> Self {
        Self::Selection(Box::new(error))
    }
}

/// Machine reports never solicit input, even when invoked from a terminal.
pub(crate) fn require_confirmation(
    format: ReportFormat,
    force: bool,
    operation: &str,
) -> io::Result<()> {
    if !force
        && (matches!(format, ReportFormat::Toml)
            || !io::stdin().is_terminal()
            || !io::stderr().is_terminal())
    {
        return Err(io::Error::other(format!(
            "{operation} requires confirmation; use --force for noninteractive execution"
        )));
    }
    Ok(())
}

/// Call only after checking interaction policy and displaying the operation's scope.
pub(crate) fn confirm_removal(force: bool, operation: &str, prompt: &str) -> io::Result<()> {
    if !force {
        write!(io::stderr().lock(), "{prompt} [y/N] ")?;
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("{operation} cancelled; nothing removed by this operation"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_log_paths_are_relative_and_remain_distinguishable() {
        let current = std::env::current_dir().unwrap();
        let mut output = Vec::new();
        for name in ["first/pkg.spec", "second/pkg.spec"] {
            write_outcome(
                &mut HumanOutput::new(&mut output, false),
                &EditOutcome::Written(current.join(name)),
            )
            .unwrap();
        }
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output,
            "[INFO] Wrote first/pkg.spec\n[INFO] Wrote second/pkg.spec\n"
        );
    }

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
            HumanOutput::new(&mut output, true)
                .message(
                    level,
                    Some(Path::new("WORK")),
                    format_args!("same plain body"),
                )
                .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(output.starts_with(color_prefix), "{output:?}");
            let (prefix, body) = output.split_once(' ').unwrap();
            assert!(prefix.contains('\u{1b}'));
            assert!(prefix.ends_with("\u{1b}[0m"), "{output:?}");
            assert_eq!(body, "WORK: same plain body\n");

            let mut output = Vec::new();
            HumanOutput::new(&mut output, false)
                .message(
                    level,
                    Some(Path::new("WORK")),
                    format_args!("same plain body"),
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
            HumanOutput::new(&mut output, false)
                .diagnostic(
                    HumanLevel::Warn,
                    start,
                    Some("RPK005"),
                    format_args!("no sha256"),
                )
                .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert_eq!(output, format!("[WARN] spec{suffix} [RPK005]: no sha256\n"));
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
        let error = HumanOutput::new(Broken, false)
            .message(HumanLevel::Warn, None, format_args!("test"))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
