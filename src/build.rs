// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Build orchestration: immutable input copies, independent engine and host, one receipt.

mod clean;
pub(crate) mod evidence;
pub(crate) mod history;
mod mock;
pub(crate) mod shell;

pub(crate) use clean::{
    CleanReport, execute as clean_result, new_report as clean_report,
    preflight as preflight_cleanup,
};

use std::{
    borrow::Cow,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Args;
use fs_err as fs;
use serde::Serialize;

use crate::environment::{Backend, Execution, compose, directory, invalid, regular_file};
use crate::output_cli::ReportFormat;

#[derive(Args)]
#[command(group(clap::ArgGroup::new("build-input").required(true).args(["work", "spec"])))]
pub(crate) struct Options {
    #[command(flatten)]
    input: crate::workspace::SpecOptions,
    /// Override Docker's current connection configuration for this invocation.
    #[arg(long, value_name = "CONTEXT")]
    context: Option<String>,
    /// Advanced: override the workspace or embedded environment with a Compose YAML file.
    #[arg(
        long,
        value_name = "FILE",
        help_heading = "Advanced options",
        hide_short_help = true
    )]
    config: Option<PathBuf>,
    /// Explicit SPEC mode only: prepared sources; symlinks and special files are rejected.
    #[arg(
        long,
        value_name = "DIR",
        value_hint = clap::ValueHint::DirPath,
        requires = "spec",
        conflicts_with = "work",
        hide_short_help = true,
        help_heading = "Advanced options"
    )]
    source_dir: Option<PathBuf>,
    /// Explicit SPEC mode only: result parent (default build); results live in `DIR/SPEC_STEM`.
    #[arg(
        long,
        value_name = "DIR",
        value_hint = clap::ValueHint::DirPath,
        requires = "spec",
        conflicts_with = "work",
        hide_short_help = true,
        help_heading = "Advanced options"
    )]
    output_dir: Option<PathBuf>,
    /// Stop after source unpacking/Patch application, or build the complete package.
    #[arg(long, value_enum, default_value_t = Stage::Build)]
    stage: Stage,
    /// Do not download source materials. Mock dependency/network access is configured separately.
    #[arg(long)]
    offline: bool,
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

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum, Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub(super) enum Stage {
    Prep,
    #[default]
    Build,
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
    fn export_patch(&self) -> Vec<String>;
}

#[derive(serde::Deserialize, Serialize)]
struct InputFile {
    // Display text only; file operations retain native paths.
    path: String,
    executable: bool,
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
    stage: Stage,
    remove_requested: bool,
    resources_retained: bool,
    session_tracking: bool,
    inputs: Vec<InputFile>,
    execution: Execution,
    engine_validation_error: Option<String>,
    success: bool,
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
                executable: crate::file_digest::executable(&fs::symlink_metadata(&path)?),
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
    let input = BuildInput::resolve(options)?;
    let spec = &input.spec;
    let source_dir = &input.source_dir;
    let output = &input.output;
    let custom_config = &input.custom_config;
    let package = &input.package;
    let root = output
        .parent()
        .ok_or_else(|| invalid("build result needs a parent"))?;
    if output.starts_with(source_dir) {
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
    let parsed = crate::spec::ParsedSpec::parse(input.source.as_str());
    let cache = input.development.as_ref().map_or_else(
        || source_dir.clone(),
        crate::workspace::Development::sources,
    );
    if crate::output_cli::debug_enabled() {
        eprintln!(
            "build: selected SPEC sha256={}",
            crate::utf8_file::sha256(&input.source)
        );
        let summary = crate::spec::inspection::Inspection::new(crate::spec::ParsedSpec::parse(
            input.source.as_str(),
        ));
        summary.write_identity(&mut io::stderr().lock())?;
        eprintln!(
            "build: SPEC {}; materials {} + {}; environment {}",
            spec.display(),
            source_dir.display(),
            cache.display(),
            custom_config
                .as_ref()
                .or(input.workspace_config.as_ref())
                .map_or_else(|| "embedded openRuyi".into(), |p| p.display().to_string())
        );
    } else {
        crate::output_cli::stderr().message(
            crate::output_cli::HumanLevel::Info,
            Some(spec),
            format_args!("build: preparing materials"),
        )?;
    }
    let materials =
        crate::check::materials::prepare(source_dir, &cache, &parsed, &[], options.offline)
            .map_err(io::Error::other)?;
    if !crate::utf8_file::is_unchanged(spec, &input.source).map_err(io::Error::other)? {
        return Err(invalid(
            "SPEC changed during material preparation; previous build retained",
        ));
    }
    eprintln!(
        "build: results and logs {}",
        crate::output_cli::human_path(output).display()
    );
    eprintln!(
        "build: {} material declarations verified{}",
        materials.paths().count(),
        if options.offline {
            " (offline materials; Mock networking unchanged)"
        } else {
            ""
        }
    );
    crate::host_process::install_handler()?;
    fs::create_dir_all(root)?;
    let previous = history::archive(output, package)?;
    fs::create_dir(output)?;
    let config = custom_config
        .clone()
        .unwrap_or_else(|| output.join(".config/compose.yaml"));
    let probe = vec![
        "python3".into(),
        "-c".into(),
        include_str!("build/probe.py").into(),
    ];
    let backend = compose::Compose {
        context: options.context.as_deref(),
        config: &config,
        remove: options.remove,
        probe: Some(&probe),
    };
    let engine = mock::Mock(options.stage);
    let staged_input = output.join("input");
    let mut inputs = Vec::new();
    let mut configuration_files = Vec::new();
    let mut _environment_lock = None;
    let execution = match input
        .stage(
            &backend,
            &engine,
            &config,
            Duration::from_secs(options.timeout),
            &mut configuration_files,
            &materials,
        )
        .and_then(|invocation| {
            if invocation.is_empty() {
                return Err(invalid("engine returned an empty invocation"));
            }
            inventory(&staged_input, &staged_input, &mut inputs)?;
            Ok(invocation)
        }) {
        Ok(invocation) => {
            let reuse = previous
                .as_ref()
                .filter(|old| old.reusable(&configuration_files));
            if let Some(old) = reuse {
                _environment_lock = Some(old.transfer(output)?);
            }
            let mut execution = backend.execute(
                reuse.map(|old| &old.receipt["execution"]["details"]),
                &invocation,
                &staged_input,
                output,
                Duration::from_secs(options.timeout),
            );
            if execution.details.is_null()
                && let Some(old) = reuse
            {
                execution.details = old.receipt["execution"]["details"].clone();
            }
            execution
        }
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
        stage: options.stage,
        remove_requested: options.remove,
        session_tracking: true,
        resources_retained: !execution.details.is_null()
            && (!options.remove || execution.cleanup_failure.is_some()),
        inputs,
        execution,
        engine_validation_error,
        success,
    };
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?;
    let receipt_path = output.join("receipt.json");
    crate::file_output::write_artifact(&receipt_path, &bytes)?;
    receipt.write_report(options, &receipt_path)?;
    Ok(success)
}

// Resolves paths once and owns the cooperative WORK lock for the complete operation.
struct BuildInput {
    package: String,
    development: Option<crate::workspace::Development>,
    spec: PathBuf,
    source: String,
    source_dir: PathBuf,
    output: PathBuf,
    custom_config: Option<PathBuf>,
    workspace_config: Option<PathBuf>,
}

impl BuildInput {
    fn resolve(options: &Options) -> io::Result<Self> {
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
        let (requested_spec, requested_sources, managed_output) = if let Some(work) =
            &options.input.work
        {
            let workspace = crate::workspace::discover()?;
            let mut area = workspace.development(work, options.input.pkgname.as_deref(), false)?;
            // Require the bound package SPEC before allocating the recipe copy.
            area.spec().map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    invalid(format!(
                        "{work}: recipe SPEC is missing; publish with: ruyipack gen {work} --apply"
                    ))
                } else {
                    error
                }
            })?;
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
            let spec = options
                .input
                .spec
                .as_ref()
                .expect("clap requires WORK or --spec");
            let sources = options
                .source_dir
                .as_ref()
                .ok_or_else(|| invalid("building an explicit SPEC requires --source-dir"))?;
            (spec.clone(), sources.clone(), None)
        };
        regular_file(&requested_spec).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound && let Some(work) = &options.input.work {
                invalid(format!("{work}: recipe SPEC is missing; publish the recipe with: ruyipack gen {work} --apply"))
            } else { error }
        })?;
        let spec = fs::canonicalize(&requested_spec)?;
        spec.file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("SPEC filename must be UTF-8"))?;
        let source_dir = directory(&requested_sources)?;
        // Named builds consume the saved PKG binding; explicit paths retain their recipe stem.
        let package = development
            .as_ref()
            .map_or_else(
                || spec.file_stem().and_then(|name| name.to_str()),
                |area| Some(area.package()),
            )
            .ok_or_else(|| invalid("SPEC stem must be UTF-8"))?;
        crate::check::metadata::Field::Name
            .validate(package)
            .map_err(invalid)?;
        let package = package.to_owned();
        let output = if let Some(output) = managed_output {
            output
        } else {
            let requested = std::env::current_dir()?.join(
                options
                    .output_dir
                    .as_deref()
                    .unwrap_or_else(|| Path::new("build")),
            );
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
            root.join(&package)
        };
        Ok(Self {
            package,
            development,
            source: crate::utf8_file::read(&spec).map_err(io::Error::other)?,
            spec,
            source_dir,
            output,
            custom_config,
            workspace_config,
        })
    }

    fn stage(
        &self,
        backend: &impl Backend,
        engine: &impl Engine,
        config: &Path,
        timeout: Duration,
        configuration_files: &mut Vec<InputFile>,
        materials: &crate::check::materials::Report,
    ) -> io::Result<Vec<String>> {
        let output = &self.output;
        let input = output.join("input");
        let spec_name = self
            .spec
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("SPEC filename must be UTF-8"))?;
        if let Some(workspace_config) = &self.workspace_config {
            // Snapshot user-owned workspace assets; normalize only these copied files.
            crate::file_tree::copy(
                workspace_config.parent().expect("absolute config"),
                &output.join(".config"),
            )?;
            inventory(output, &output.join(".config"), configuration_files)?;
        } else if self.custom_config.is_none() {
            crate::environment::write(&output.join(".config"))?;
            inventory(output, &output.join(".config"), configuration_files)?;
        } else {
            let content = crate::file_digest::read(config).map_err(io::Error::other)?;
            configuration_files.push(InputFile {
                executable: crate::file_digest::executable(&fs::symlink_metadata(config)?),
                path: config.to_string_lossy().into_owned(),
                content,
            });
        }
        let parsed = crate::spec::ParsedSpec::parse(self.source.as_str());
        if !crate::utf8_file::is_unchanged(&self.spec, &self.source).map_err(io::Error::other)? {
            return Err(invalid("SPEC changed before staging"));
        }
        backend.validate()?;
        fs::create_dir(&input)?;
        fs::create_dir(input.join("SPECS"))?;
        fs::write(input.join("SPECS").join(spec_name), &self.source)?;
        crate::file_tree::copy(&self.source_dir, &input.join("SOURCES"))?;
        for (path, name) in materials.paths() {
            if path != self.source_dir.join(name) {
                fs::copy(path, input.join("SOURCES").join(name))?;
            }
        }
        let staged =
            crate::check::materials::analyze(Some(&input.join("SOURCES")), None, &parsed, &[]);
        if !staged.valid {
            return Err(invalid(
                "staged materials failed verification; run check --materials for details",
            ));
        }
        engine.stage(&input, spec_name, timeout)
    }
}

impl Receipt<'_> {
    fn write_report(&self, options: &Options, receipt_path: &Path) -> io::Result<()> {
        let directory = receipt_path.parent().expect("receipt directory");
        let mut next_steps = std::collections::BTreeMap::new();
        let invocation: Vec<String> = std::env::args().collect();
        let program = invocation[0].clone();
        next_steps.insert("retry", invocation);
        let mut clean = vec![program.clone(), "clean".into()];
        if let Some(work) = &options.input.work {
            clean.push(work.clone());
        } else {
            clean.extend([
                "--build-dir".into(),
                directory.to_string_lossy().into_owned(),
            ]);
        }
        next_steps.insert("clean", clean);
        if self.resources_retained && self.execution.details["container_id"].is_string() {
            let mut shell = vec![program, "shell".into()];
            if let Some(work) = &options.input.work {
                shell.push(work.clone());
            } else {
                shell.extend([
                    self.package.into(),
                    "--output-dir".into(),
                    directory
                        .parent()
                        .expect("result parent")
                        .to_string_lossy()
                        .into_owned(),
                ]);
            }
            next_steps.insert("shell", shell);
        }
        let report = BuildReport {
            format_version: 1,
            tool: crate::tool::identity(),
            operation: "build",
            stage: self.stage,
            success: self.success,
            receipt: receipt_path.to_string_lossy(),
            failure: self.execution.failure.as_deref(),
            artifact_error: self.execution.artifact_error.as_deref(),
            engine_validation_error: self.engine_validation_error.as_deref(),
            cleanup_failure: self.execution.cleanup_failure.as_deref(),
            cleanup_skipped: self.execution.cleanup_skipped,
            logs: [directory.join("host"), directory.join("engine")]
                .into_iter()
                .filter(|path| path.is_dir())
                .collect(),
            artifacts: self.success.then(|| directory.join("engine")),
            working_directory: std::env::current_dir()?,
            next_steps,
        };
        match options.format {
            ReportFormat::Toml => crate::report::write(&mut io::stdout().lock(), &report),
            ReportFormat::Human => report.write_human(),
        }
    }
}

impl BuildReport<'_> {
    fn write_human(&self) -> io::Result<()> {
        use crate::output_cli::{HumanLevel, human_path, stderr};
        let mut out = stderr();
        out.message(
            if self.success {
                HumanLevel::Info
            } else {
                HumanLevel::Error
            },
            None,
            format_args!(
                "{} {}",
                match self.stage {
                    Stage::Prep => "prep",
                    Stage::Build => "build",
                },
                if self.success { "completed" } else { "failed" }
            ),
        )?;
        for (stage, error) in [
            ("execution", self.failure),
            ("artifact retrieval", self.artifact_error),
            ("result verification", self.engine_validation_error),
            ("cleanup", self.cleanup_failure),
        ] {
            if let Some(error) = error {
                out.message(HumanLevel::Error, None, format_args!("{stage}: {error}"))?;
            }
        }
        for path in &self.logs {
            out.message(
                HumanLevel::Info,
                None,
                format_args!("logs: {}", human_path(path).display()),
            )?;
        }
        if let Some(path) = &self.artifacts {
            out.message(
                HumanLevel::Info,
                None,
                format_args!("artifacts: {}", human_path(path).display()),
            )?;
        }
        out.message(
            HumanLevel::Debug,
            None,
            format_args!("receipt: {}", self.receipt),
        )?;
        for (action, argv) in &self.next_steps {
            if *action == "retry" && self.success {
                continue;
            }
            let command = argv
                .iter()
                .map(|arg| shell_words::quote(arg))
                .collect::<Vec<_>>()
                .join(" ");
            out.message(HumanLevel::Info, None, format_args!("{action}: {command}"))?;
        }
        Ok(())
    }
}

/// CLI outcome points to complete retained evidence; engine/host storage has its own protocol.
#[derive(Serialize)]
struct BuildReport<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    stage: Stage,
    operation: &'static str,
    success: bool,
    receipt: Cow<'a, str>,
    logs: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<PathBuf>,
    working_directory: PathBuf,
    next_steps: std::collections::BTreeMap<&'static str, Vec<String>>,
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
