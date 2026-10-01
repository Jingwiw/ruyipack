// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Build orchestration: immutable input copies, independent engine and host, one receipt.

mod compose;
mod mock;
mod process;
pub(crate) mod shell;

pub(crate) use compose::clean::run as clean_result;

use std::{
    borrow::Cow,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Args;
use fs_err as fs;
use serde::Serialize;

use crate::output_cli::ReportFormat;

#[derive(Args)]
pub(crate) struct Options {
    /// Workspace development area; the package defaults to WORK on first use.
    #[arg(
        value_name = "WORK",
        required_unless_present = "spec",
        conflicts_with = "spec"
    )]
    work: Option<String>,
    /// Advanced: build a prepared SPEC without resolving a managed development area.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["work", "pkgname"])]
    spec: Option<PathBuf>,
    /// Package binding for a new named area; existing areas cannot be rebound.
    #[arg(long, value_name = "PKG", requires = "work")]
    pkgname: Option<String>,
    /// Override Docker's current connection configuration for this invocation.
    #[arg(long, value_name = "CONTEXT")]
    context: Option<String>,
    /// Advanced: override the workspace or embedded environment with a Compose YAML file.
    #[arg(long, value_name = "FILE", help_heading = "Advanced options")]
    config: Option<PathBuf>,
    /// Explicit SPEC mode only: prepared sources; symlinks and special files are rejected.
    #[arg(long, value_name = "DIR", requires = "spec", conflicts_with = "work")]
    source_dir: Option<PathBuf>,
    /// Explicit SPEC mode only: result parent (default build); results live in DIR/SPEC_STEM.
    #[arg(long, value_name = "DIR", requires = "spec", conflicts_with = "work")]
    dir: Option<PathBuf>,
    /// Whole backend execution deadline; bounded recovery runs separately afterward.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    /// Remove this build's worker after retrieving evidence; otherwise leave it stopped.
    #[arg(long = "rm")]
    remove: bool,
    /// Selects stdout presentation; the complete retained receipt is always written.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

/// The engine owns command arguments; the backend transports them unchanged.
trait Engine {
    fn name(&self) -> &'static str;
    fn stage(
        &self,
        input_dir: &Path,
        spec_name: &str,
        timeout: Duration,
    ) -> io::Result<Vec<String>>;
    fn verify_result(&self, output: &Path) -> io::Result<()>;
    fn shell(&self) -> Vec<String>;
}

trait Backend {
    fn name(&self) -> &'static str;
    fn validate(&self) -> io::Result<()>;
    fn shell(
        &self,
        output: &Path,
        invocation: &[String],
        resources: &serde_json::Value,
        timeout: Duration,
    ) -> io::Result<bool>;
    fn execute(
        &self,
        invocation: &[String],
        staged_input: &Path,
        output: &Path,
        timeout: Duration,
    ) -> Execution;
}

#[derive(Serialize)]
struct CommandRecord {
    argv: Vec<String>,
    started: bool,
    exit_code: Option<i32>,
    exit_status: Option<String>,
    timed_out: bool,
    interrupted: bool,
    elapsed_ms: u128,
    stdout: String,
    stderr: String,
    error: Option<String>,
    termination: Option<crate::host_process::Termination>,
}

#[derive(Serialize, Default)]
struct Execution {
    success: bool,
    details: serde_json::Value,
    failure: Option<String>,
    artifact_error: Option<String>,
    cleanup_failure: Option<String>,
    cleanup_skipped: bool,
    recovery_commands: Vec<Vec<String>>,
    commands: Vec<CommandRecord>,
}

impl Execution {
    fn failed(message: String) -> Self {
        Self {
            failure: Some(message),
            ..Self::default()
        }
    }
}

#[derive(Serialize)]
struct InputFile {
    // Display text only; file operations retain native paths.
    path: String,
    #[serde(flatten)]
    content: crate::file_digest::Content,
}

#[derive(Serialize)]
struct Receipt<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    backend: &'static str,
    engine: &'static str,
    context: Option<&'a str>,
    // These are display paths, not a replayable file identity.
    config: Cow<'a, str>,
    configuration_files: Vec<InputFile>,
    package: &'a str,
    spec: Cow<'a, str>,
    source_dir: Cow<'a, str>,
    timeout_seconds: u64,
    remove_requested: bool,
    inputs: Vec<InputFile>,
    execution: Execution,
    engine_validation_error: Option<String>,
    success: bool,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn regular_file(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(invalid(format!(
            "{}: expected a regular file, not a symlink or special file",
            path.display()
        )));
    }
    Ok(())
}

fn directory(path: &Path) -> io::Result<PathBuf> {
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(invalid(format!(
            "{}: expected a directory, not a symlink",
            path.display()
        )));
    }
    fs::canonicalize(path)
}

fn copy_sources(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir(destination)?;
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs_err::DirEntry::file_name);
    for entry in entries {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_sources(&from, &to)?;
        } else if kind.is_file() {
            fs::copy(&from, &to)?;
        } else {
            return Err(invalid(format!(
                "{}: source inputs must not contain symlinks or special files",
                from.display()
            )));
        }
    }
    Ok(())
}

fn inventory(root: &Path, directory: &Path, files: &mut Vec<InputFile>) -> io::Result<()> {
    normalize_mode(directory, &fs::symlink_metadata(directory)?)?;
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs_err::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            inventory(root, &path, files)?;
        } else {
            normalize_mode(&path, &fs::symlink_metadata(&path)?)?;
            let content = crate::file_digest::read(&path).map_err(io::Error::other)?;
            files.push(InputFile {
                path: path
                    .strip_prefix(root)
                    .expect("staged child")
                    .to_string_lossy()
                    .into_owned(),
                content,
            });
        }
    }
    Ok(())
}

fn normalize_mode(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(invalid(format!(
            "{}: staged input is not a regular file or directory",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if metadata.is_dir() {
            0o755
        } else {
            0o644 | (metadata.permissions().mode() & 0o111)
        };
        fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    if options
        .context
        .as_deref()
        .is_some_and(|context| context.trim().is_empty())
    {
        return Err(invalid("--context must not be empty"));
    }
    let custom_config = options
        .config
        .as_ref()
        .map(|path| {
            regular_file(path)?;
            fs::canonicalize(path)
        })
        .transpose()?;
    // Keep the binding's cooperative lock through staging, execution and receipt publication.
    let mut development = None;
    let mut workspace_config = None;
    let (requested_spec, requested_sources, managed_output) = if let Some(work) = &options.work {
        let workspace = crate::workspace::discover()?;
        let mut area = workspace.development(work, options.pkgname.as_deref(), false)?;
        // Require the bound package SPEC before allocating the Git checkout.
        area.spec()?;
        if options.config.is_none() {
            let config = workspace.build_config();
            regular_file(&config)?;
            workspace_config = Some(fs::canonicalize(config)?);
        }
        area.create()?;
        let spec = area.spec()?;
        let sources = area.package_directory().to_path_buf();
        let output = area.directory().join("build");
        development = Some(area);
        (spec, sources, Some(output))
    } else {
        let sources = options
            .source_dir
            .as_ref()
            .ok_or_else(|| invalid("building an explicit SPEC requires --source-dir"))?;
        (
            options
                .spec
                .as_ref()
                .expect("clap requires WORK or --spec")
                .clone(),
            sources.clone(),
            None,
        )
    };
    regular_file(&requested_spec)?;
    let spec = fs::canonicalize(&requested_spec)?;
    let spec_name = spec
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("SPEC filename must be UTF-8"))?;
    let source_dir = directory(&requested_sources)?;
    // Named builds consume the saved PKG binding; explicit paths retain their recipe stem.
    let package = match &development {
        Some(area) => area.package(),
        None => spec
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("SPEC stem must be UTF-8"))?,
    };
    crate::check::metadata::Field::Name
        .validate(package)
        .map_err(invalid)?;
    let output = match managed_output {
        Some(output) => output,
        None => {
            let requested = std::env::current_dir()?
                .join(options.dir.as_deref().unwrap_or_else(|| Path::new("build")));
            let root = if requested.try_exists()? {
                directory(&requested)?
            } else {
                fs::canonicalize(
                    requested
                        .parent()
                        .ok_or_else(|| invalid("build directory needs a parent"))?,
                )?
                .join(
                    requested
                        .file_name()
                        .ok_or_else(|| invalid("build directory needs a name"))?,
                )
            };
            root.join(package)
        }
    };
    let root = output
        .parent()
        .ok_or_else(|| invalid("build result needs a parent"))?;
    if output.starts_with(&source_dir) {
        return Err(invalid("build directory must not be inside --source-dir"));
    }
    if custom_config
        .as_ref()
        .is_some_and(|path| output.starts_with(path.parent().expect("absolute config")))
    {
        return Err(invalid(
            "build directory must not be inside the custom config directory",
        ));
    }
    crate::host_process::install_handler()?;
    fs::create_dir_all(root)?;
    fs::create_dir(&output).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            invalid(format!(
                "{} already exists; use shell, or explicitly clean this retained result before another build",
                output.display()
            ))
        } else { error }
    })?;
    let config = custom_config
        .clone()
        .unwrap_or_else(|| output.join(".config/compose.yaml"));
    let backend = compose::Compose {
        context: options.context.as_deref(),
        config: &config,
        remove: options.remove,
    };
    let engine = mock::Mock;
    let input = output.join("input");
    let mut inputs = Vec::new();
    let mut configuration_files = Vec::new();
    let mut stage = || -> io::Result<Vec<String>> {
        if let Some(workspace_config) = &workspace_config {
            // Snapshot user-owned workspace assets; normalize only these copied files.
            copy_sources(
                workspace_config.parent().expect("absolute config"),
                &output.join(".config"),
            )?;
            inventory(&output, &output.join(".config"), &mut configuration_files)?;
        } else if custom_config.is_none() {
            crate::environment::write(&output.join(".config"))?;
            inventory(&output, &output.join(".config"), &mut configuration_files)?;
        } else {
            let content = crate::file_digest::read(&config).map_err(io::Error::other)?;
            configuration_files.push(InputFile {
                path: config.to_string_lossy().into_owned(),
                content,
            });
        }
        backend.validate()?;
        fs::create_dir(&input)?;
        fs::create_dir(input.join("SPECS"))?;
        fs::copy(&spec, input.join("SPECS").join(spec_name))?;
        copy_sources(&source_dir, &input.join("SOURCES"))?;
        engine.stage(&input, spec_name, Duration::from_secs(options.timeout))
    };
    let execution = match stage().and_then(|invocation| {
        if invocation.is_empty() {
            return Err(invalid("engine returned an empty invocation"));
        }
        inventory(&input, &input, &mut inputs)?;
        Ok(invocation)
    }) {
        Ok(invocation) => backend.execute(
            &invocation,
            &input,
            &output,
            Duration::from_secs(options.timeout),
        ),
        Err(error) => Execution::failed(format!("input staging failed: {error}")),
    };
    let engine_validation_error =
        if execution.failure.is_none() && execution.artifact_error.is_none() {
            engine
                .verify_result(&output.join("engine"))
                .err()
                .map(|error| error.to_string())
        } else {
            None
        };
    let success = execution.success && engine_validation_error.is_none();
    let receipt = Receipt {
        format_version: 1,
        tool: crate::tool::identity(),
        backend: backend.name(),
        engine: engine.name(),
        context: options.context.as_deref(),
        config: config.to_string_lossy(),
        configuration_files,
        package,
        spec: spec.to_string_lossy(),
        source_dir: source_dir.to_string_lossy(),
        timeout_seconds: options.timeout,
        remove_requested: options.remove,
        inputs,
        execution,
        engine_validation_error,
        success,
    };
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?;
    let receipt_path = output.join("receipt.json");
    fs::write(&receipt_path, bytes)?;
    match options.format {
        ReportFormat::Toml => {
            crate::report::write(
                &mut io::stdout().lock(),
                &BuildReport {
                    format_version: 1,
                    tool: crate::tool::identity(),
                    operation: "build",
                    success,
                    receipt: receipt_path.to_string_lossy(),
                    failure: receipt.execution.failure.as_deref(),
                    artifact_error: receipt.execution.artifact_error.as_deref(),
                    engine_validation_error: receipt.engine_validation_error.as_deref(),
                    cleanup_failure: receipt.execution.cleanup_failure.as_deref(),
                    cleanup_skipped: receipt.execution.cleanup_skipped,
                },
            )?;
        }
        ReportFormat::Human => {
            writeln!(
                io::stdout().lock(),
                "build {}: {}",
                if success { "completed" } else { "failed" },
                receipt_path.display()
            )?;
            if receipt.execution.cleanup_skipped {
                writeln!(
                    io::stderr().lock(),
                    "automatic removal not requested; resource state and recovery commands: {}",
                    receipt_path.display()
                )?;
            }
            if let Some(error) = &receipt.execution.failure {
                writeln!(io::stderr().lock(), "{error}")?;
            }
            if let Some(error) = &receipt.execution.artifact_error {
                writeln!(io::stderr().lock(), "artifact retrieval: {error}")?;
            }
            if let Some(error) = &receipt.engine_validation_error {
                writeln!(io::stderr().lock(), "engine result verification: {error}")?;
            }
            if let Some(error) = &receipt.execution.cleanup_failure {
                writeln!(io::stderr().lock(), "cleanup: {error}")?;
            }
        }
    }
    Ok(success)
}

/// CLI outcome points to complete retained evidence; engine/host storage has its own protocol.
#[derive(Serialize)]
struct BuildReport<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    operation: &'static str,
    success: bool,
    receipt: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_validation_error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cleanup_failure: Option<&'a str>,
    cleanup_skipped: bool,
}
