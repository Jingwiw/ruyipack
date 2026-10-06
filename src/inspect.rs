// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only SPEC facts and source-mapped editable projections.

use clap::Args;
use std::io::{self, Write};

use crate::{
    output_cli::{ReportError, ReportFormat, report_input},
    spec::{ParsedSpec, document::Snapshot, inspection::Inspection},
    workspace::SpecOptions,
};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("inspect-input").required(true).args(["work", "spec"])))]
#[command(group(clap::ArgGroup::new("projection-scope").args(["fields", "all"])))]
pub(crate) struct Options {
    #[command(flatten)]
    pub(crate) input: SpecOptions,
    /// Show parser facts as human-readable text or a machine report.
    #[arg(long, value_enum, default_value = "human")]
    pub(crate) format: ReportFormat,
    /// Print the editable TOML projection without editing or validating package policy.
    #[arg(long, requires = "projection-scope", conflicts_with = "format")]
    pub(crate) editable: bool,
    /// Select an editable field or group; repeat to add fields.
    #[arg(
        long = "field",
        value_name = "FIELD",
        requires = "editable",
        conflicts_with = "all"
    )]
    pub(crate) fields: Vec<String>,
    /// Include all supported editable fields; unsupported constructs are rejected.
    #[arg(long, requires = "editable")]
    pub(crate) all: bool,
}

/// Read-only projection rejects ambiguous mappings; ordinary inspection retains diagnostics.
pub(crate) fn run(options: &Options) -> Result<bool, ReportError> {
    let resolved = options.input.resolve();
    if !options.editable
        && resolved
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        && let Some(work) = &options.input.work
        && let Ok(area) =
            crate::workspace::discover().and_then(|workspace| workspace.existing_development(work))
    {
        WorkReport::collect(&area, None)
            .write(options.format, &mut io::stdout().lock())
            .map_err(ReportError::Stdout)?;
        return Ok(true);
    }
    let Some(input) = report_input(resolved, &options.input.display(), options.format)? else {
        return Ok(false);
    };
    let parsed = ParsedSpec::parse(&input.source);
    let mut output = io::stdout().lock();
    if options.editable {
        let snapshot = Snapshot::capture_selected(&parsed, &options.fields).map_err(|error| {
            ReportError::Projection(format!("{}: {error}", input.path.display()))
        })?;
        let document = toml::to_string_pretty(snapshot.document())
            .map_err(|error| ReportError::Projection(error.to_string()))?;
        output
            .write_all(document.as_bytes())
            .map_err(ReportError::Stdout)?;
    } else {
        let view = Inspection::new(parsed);
        match options.format {
            ReportFormat::Human => {
                if let Some(area) = input.development() {
                    WorkReport::collect(area, Some(&input.source))
                        .write(ReportFormat::Human, &mut output)
                        .map_err(ReportError::Stdout)?;
                }
                view.write_diagnostics(&input.path, &mut crate::output_cli::stderr())
                    .map_err(ReportError::Stderr)?;
                view.write_human(&mut output)
            }
            ReportFormat::Toml => view
                .write_toml(&input.path, input.revision.as_deref(), &mut output)
                .and_then(|()| {
                    if let Some(area) = input.development() {
                        WorkReport::collect(area, Some(&input.source))
                            .write(ReportFormat::Toml, &mut output)
                    } else {
                        Ok(())
                    }
                }),
        }
        .map_err(ReportError::Stdout)?;
    }
    Ok(true)
}

/// WORK state is collected once; presentation never selects a different operation.
#[derive(serde::Serialize)]
struct WorkReport {
    work: WorkState,
}

#[derive(serde::Serialize)]
struct WorkState {
    package: String,
    spec_present: bool,
    authoring: std::path::PathBuf,
    authoring_present: bool,
    recipe: std::path::PathBuf,
    recipe_present: bool,
    stage_present: bool,
    stage_applied_by_build: bool,
    receipt: Option<std::path::PathBuf>,
    last_build_success: Option<bool>,
    spec_matches_last_build: Option<bool>,
    receipt_error: Option<String>,
    builds: Vec<crate::build::history::Attempt>,
    builds_error: Option<String>,
}

impl WorkReport {
    fn collect(area: &crate::workspace::Development, source: Option<&str>) -> Self {
        let mut work = WorkState {
            package: area.package().to_owned(),
            spec_present: source.is_some(),
            authoring: area.manifest(),
            authoring_present: area.manifest().is_file(),
            recipe: area.package_directory().to_owned(),
            recipe_present: area.package_directory().is_dir(),
            stage_present: area.directory().join("stage").is_dir(),
            stage_applied_by_build: false,
            receipt: None,
            last_build_success: None,
            spec_matches_last_build: None,
            receipt_error: None,
            builds: Vec::new(),
            builds_error: None,
        };
        match crate::build::history::list(area.directory()) {
            Ok(builds) => work.builds = builds,
            Err(error) => work.builds_error = Some(error.to_string()),
        }
        let path = area.directory().join("build/receipt.json");
        if path.exists() {
            work.receipt = Some(path.clone());
            match fs_err::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| e.to_string())
                }) {
                Ok(receipt) => {
                    work.last_build_success = receipt["success"].as_bool();
                    let spec = area.spec().ok().and_then(|path| {
                        path.file_name()
                            .map(|name| format!("SPECS/{}", name.to_string_lossy()))
                    });
                    let hash = receipt["inputs"]
                        .as_array()
                        .and_then(|files| {
                            files.iter().find(|file| {
                                spec.as_deref().is_some_and(|spec| file["path"] == spec)
                            })
                        })
                        .and_then(|file| file["sha256"].as_str());
                    work.spec_matches_last_build = source
                        .zip(hash)
                        .map(|(source, expected)| crate::utf8_file::sha256(source) == expected);
                }
                Err(error) => work.receipt_error = Some(error),
            }
        }
        Self { work }
    }

    fn write(&self, format: ReportFormat, output: &mut impl Write) -> io::Result<()> {
        if matches!(format, ReportFormat::Toml) {
            return crate::report::write(output, self);
        }
        let work = &self.work;
        writeln!(output, "Package: {}", work.package)?;
        writeln!(
            output,
            "Authoring: {} (present: {})",
            crate::output_cli::human_path(&work.authoring).display(),
            work.authoring_present
        )?;
        writeln!(
            output,
            "Recipe: {} (present: {})",
            crate::output_cli::human_path(&work.recipe).display(),
            work.recipe_present
        )?;
        writeln!(output, "SPEC present: {}", work.spec_present)?;
        writeln!(output, "Edit stage present: {}", work.stage_present)?;
        if let Some(path) = &work.receipt {
            writeln!(
                output,
                "Last build: {}",
                crate::output_cli::human_path(path).display()
            )?;
            writeln!(
                output,
                "Last result: {}; SPEC matches recorded input: {} (materials and environment not compared)",
                work.last_build_success
                    .map_or("unknown", |v| if v { "passed" } else { "failed" }),
                work.spec_matches_last_build
                    .map_or("unknown", |v| if v { "yes" } else { "no" })
            )?;
        }
        for build in &work.builds {
            writeln!(
                output,
                "Build {}: result={}, retained={}, session={}, target={}, daemon={}, logs={}",
                build.id,
                build
                    .success
                    .map_or("unknown", |v| if v { "passed" } else { "failed" }),
                build
                    .resources_retained
                    .map_or("unknown", |v| if v { "yes" } else { "no" }),
                build
                    .environment_may_have_changed
                    .map_or("unknown", |v| if v { "used" } else { "unused" }),
                build.target_arch.as_deref().unwrap_or("unknown"),
                build.daemon_arch.as_deref().unwrap_or("unknown"),
                crate::output_cli::human_path(&build.directory).display()
            )?;
            if let Some(error) = &build.error {
                writeln!(output, "Build unavailable: {error}")?;
            }
        }
        if let Some(error) = &work.builds_error {
            writeln!(output, "Build history unavailable: {error}")?;
        }
        if let Some(error) = &work.receipt_error {
            writeln!(output, "Last result unavailable: {error}")?;
        }
        Ok(())
    }
}
