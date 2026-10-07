// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared static SPEC checks, rule selection, and the check command boundary.

pub(crate) mod build;
pub(crate) mod directory;
pub(crate) mod license;
pub(crate) mod materials;
pub(crate) mod metadata;
pub(crate) mod upgrade;

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::{
    check_report::{CheckReport, Finding, IncompleteReason, SelectedRule, Severity},
    output_cli::{ReportError, ReportFormat},
    parser_diagnostic,
    spec::ParsedSpec,
    workspace::SpecOptions,
};

/// Findings and unfinished checks are independent facts, not severity conventions.
#[derive(Default)]
pub(crate) struct RuleResult {
    pub(crate) source_uncertainty: Option<String>,
    pub(crate) findings: Vec<Finding>,
    pub(crate) incomplete_reasons: Vec<IncompleteReason>,
}

// Missing or malformed source digests warn while authoring, so unrelated edits
// remain possible. Authoring a digest still requires a valid SHA-256.
pub(crate) const SOURCE_DIGEST_RULE: SelectedRule = SelectedRule {
    code: "RPK005",
    severity: Severity::Warn,
};

pub(crate) const FILE_LIST_RULE: SelectedRule = SelectedRule {
    code: "RPK008",
    severity: Severity::Warn,
};

/// Purpose changes admission, not field identity or editing safety.
#[derive(Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Policy {
    #[default]
    Authoring,
    Submit,
}

impl Policy {
    fn severity(self, code: &str, severity: Severity) -> Severity {
        match (self, code) {
            (Self::Submit, "RPK005" | "RPK006" | "RPK007" | "RPK008") => Severity::Deny,
            _ => severity,
        }
    }

    fn apply(self, rules: &mut [SelectedRule], result: &mut RuleResult) {
        for rule in rules {
            rule.severity = self.severity(rule.code, rule.severity);
        }
        for finding in &mut result.findings {
            finding.severity = self.severity(finding.code, finding.severity);
        }
        if self == Self::Submit
            && result.source_uncertainty.is_some()
            && !result
                .incomplete_reasons
                .contains(&IncompleteReason::ParserError)
        {
            result
                .incomplete_reasons
                .push(IncompleteReason::UnresolvedSources);
        }
    }
}

/// Runs selected static checks; neither policy verifies downloaded bytes or builds.
pub(crate) fn analyze(spec: &ParsedSpec<'_>, policy: Policy, defines: &[String]) -> CheckReport {
    let source = spec.source();
    let diagnostics = spec.diagnostics();

    let parser_error = diagnostics
        .iter()
        .any(|item| item.severity == parser_diagnostic::Severity::Error);
    let (mut selected_rules, mut findings) =
        crate::spec::analyzer::required_tags((!parser_error).then_some(spec));
    selected_rules.push(license::RULE);
    selected_rules.extend(metadata::RULES);
    selected_rules.push(build::RULE);
    selected_rules.push(SOURCE_DIGEST_RULE);
    selected_rules.push(crate::spec::policy_fix::RULE);
    selected_rules.push(FILE_LIST_RULE);
    let mut result = if parser_error {
        RuleResult {
            incomplete_reasons: vec![IncompleteReason::ParserError],
            source_uncertainty: Some("parser errors prevent Source resolution".into()),
            ..RuleResult::default()
        }
    } else {
        spec.policy_checks(defines)
    };
    findings.append(&mut result.findings);
    result.findings = findings;
    policy.apply(&mut selected_rules, &mut result);
    CheckReport::analyzed(source, selected_rules, diagnostics, result, policy, defines)
}

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("check-work").args(["work", "plan"])), group(clap::ArgGroup::new("check-input").args(["work", "spec", "manifest", "directory", "plan"]).required(true)))]
pub(crate) struct Options {
    #[command(flatten)]
    directory: directory::Options,
    #[command(flatten)]
    input: SpecOptions,
    /// Check each WORK in a plan; repeat to combine package selections.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["work", "spec", "manifest", "directory", "pkgname", "source_dir", "repology_project"])]
    plan: Vec<PathBuf>,
    /// Apply supported metadata, formatting and Source repairs in WORK.
    #[arg(long, requires = "check-work", conflicts_with_all = ["manifest", "spec", "materials", "policy"])]
    auto_fix: bool,
    /// Also report Repology versions; only --auto-fix applies simple, patch-free upgrades.
    #[arg(long, conflicts_with = "manifest")]
    upgrade: bool,
    /// Override the Repology project identity, for example python:requests.
    #[arg(long, requires = "upgrade")]
    repology_project: Option<String>,
    /// Check an authoring manifest by rendering it in memory, without downloads or writes.
    #[arg(long, conflicts_with_all = ["work", "spec", "pkgname", "defines"],
        value_name = "PATH")]
    manifest: Option<PathBuf>,
    /// Also check staged Source/Patch files and report sizes and SHA-256 digests.
    #[arg(long)]
    materials: bool,
    /// Prepared RPM _sourcedir; defaults to the input directory, but must be explicit without local recipe files.
    #[arg(long, requires = "materials", value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
    source_dir: Option<PathBuf>,
    /// Static admission policy; neither policy verifies native builds.
    #[arg(long, value_enum, default_value_t = Policy::Authoring)]
    policy: Policy,
    /// Define a static Source/Patch macro; other rules still check unevaluated syntax.
    #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
    defines: Vec<String>,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

enum Checked {
    Spec {
        path: PathBuf,
        report: Box<CheckReport>,
    },
    Edit(Box<crate::edit::Operation>),
    ManifestError {
        path: PathBuf,
        sha256: String,
        error: crate::render::RenderError,
    },
}

impl serde::Serialize for Checked {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Spec { path, report } => {
                serde::Serialize::serialize(&report.structured(path), serializer)
            }
            Self::Edit(result) => serde::Serialize::serialize(result, serializer),
            Self::ManifestError {
                path,
                sha256,
                error,
            } => serde::Serialize::serialize(
                &crate::report::failed(
                    crate::report::Input {
                        display_path: path.to_string_lossy(),
                        sha256: Some(sha256),
                        revision: None,
                    },
                    crate::report::failure("invalid-manifest", error),
                ),
                serializer,
            ),
        }
    }
}

impl Checked {
    fn success(&self) -> bool {
        match self {
            Self::Spec { report, .. } => report.is_success(),
            Self::Edit(result) => result.success(),
            Self::ManifestError { .. } => false,
        }
    }
    fn print(self, format: ReportFormat) -> Result<bool, ReportError> {
        let success = self.success();
        if matches!(self, Self::ManifestError { .. }) && matches!(format, ReportFormat::Toml) {
            crate::report::write(&mut io::stdout().lock(), &self).map_err(ReportError::Stdout)?;
            return Ok(false);
        }
        match self {
            Self::ManifestError { error, .. } => return Err(ReportError::Manifest(error)),
            Self::Spec { path, report } => write_report(&report, &path, format)?,
            Self::Edit(result) => {
                return result
                    .print()
                    .map_err(|e| ReportError::Projection(e.to_string()));
            }
        }
        Ok(success)
    }
}

pub(crate) fn run(options: &Options) -> Result<bool, ReportError> {
    #[derive(serde::Serialize)]
    struct Outcome {
        work: String,
        success: bool,
        result: Option<Checked>,
        error: Option<String>,
    }
    if options.directory.directory.is_some() {
        return directory::run(&options.directory, options.format);
    }
    if options.plan.is_empty() {
        let display = options.manifest.as_ref().map_or_else(
            || options.input.display(),
            |p| p.to_string_lossy().into_owned(),
        );
        let Some(result) = crate::output_cli::report_input(
            evaluate(options, &options.input),
            &display,
            options.format,
        )?
        else {
            return Ok(false);
        };
        return result.print(options.format);
    }
    let Some(plans) =
        crate::output_cli::report_input(crate::plan::load(&options.plan), "plan", options.format)?
    else {
        return Ok(false);
    };
    let works: Vec<_> = plans
        .into_iter()
        .flat_map(|p| p.packages.into_iter().map(|t| t.work))
        .collect();
    let report = crate::batch::run(
        "check",
        &works,
        |work| {
            let input = SpecOptions {
                work: Some(work.clone()),
                spec: None,
                pkgname: None,
            };
            let result = evaluate(options, &input);
            let outcome = match result {
                Ok(result) => Outcome {
                    work: work.clone(),
                    success: result.success(),
                    result: Some(result),
                    error: None,
                },
                Err(error) => Outcome {
                    work: work.clone(),
                    success: false,
                    result: None,
                    error: Some(error.to_string()),
                },
            };
            std::ops::ControlFlow::Continue(outcome)
        },
        |outcome| outcome.success,
    );
    match options.format {
        ReportFormat::Toml => {
            crate::report::write(&mut io::stdout().lock(), &report).map_err(ReportError::Stdout)?;
        }
        ReportFormat::Human => {
            for outcome in report.results {
                if let Some(result) = outcome.result {
                    if let Err(error) = result.print(options.format) {
                        if matches!(error, ReportError::Stdout(_) | ReportError::Stderr(_)) {
                            return Err(error);
                        }
                        crate::output_cli::stderr().message(
                            crate::output_cli::HumanLevel::Error,
                            Some(Path::new(&outcome.work)),
                            format_args!("{error}"),
                        )?;
                    }
                } else if let Some(error) = outcome.error {
                    crate::output_cli::stderr().message(
                        crate::output_cli::HumanLevel::Error,
                        Some(Path::new(&outcome.work)),
                        format_args!("{error}"),
                    )?;
                }
            }
        }
    }
    Ok(report.success)
}

fn evaluate(options: &Options, input: &SpecOptions) -> Result<Checked, ReportError> {
    if options.auto_fix {
        let upgrade = if options.upgrade {
            let input = input
                .resolve()
                .map_err(|e| ReportError::Projection(e.to_string()))?;
            Some(upgrade::query(
                &input.source,
                options.repology_project.as_deref(),
            ))
        } else {
            None
        };
        if matches!(options.format, ReportFormat::Human)
            && let Some(reason) = upgrade.as_ref().and_then(|report| report.error.as_deref())
        {
            crate::output_cli::stderr().message(
                crate::output_cli::HumanLevel::Warn,
                Some(Path::new(&input.display())),
                format_args!("upgrade not applied: {reason}; continuing basic fixes"),
            )?;
        }
        return Ok(Checked::Edit(Box::new(crate::edit::evaluate(
            crate::edit::Options {
                works: vec![input.work.clone().expect("auto-fix requires WORK")],
                pkgname: input.pkgname.clone(),
                hash: true,
                repair_missing: true,
                set: upgrade
                    .as_ref()
                    .filter(|r| r.applies_candidate())
                    .and_then(|r| r.candidate.as_ref())
                    .map(|v| vec![("package.version".into(), v.clone())])
                    .unwrap_or_default(),
                expect_sha256: upgrade.as_ref().map(|r| r.input_sha256.clone()),
                upgrade: upgrade.map(Box::new),

                apply: true,
                defines: options.defines.clone(),
                format: Some(options.format),
                ..Default::default()
            },
        ))));
    }
    let spec_input = if options.manifest.is_none() {
        let input = input
            .resolve()
            .map_err(|e| ReportError::Projection(e.to_string()))?;
        Some(input)
    } else {
        None
    };
    let manifest_source;
    let (path, original) = if let Some(input) = &spec_input {
        (input.path.as_path(), input.source.as_str())
    } else {
        let path = options.manifest.as_ref().expect("SPEC or manifest input");
        let source =
            crate::utf8_file::read(path).map_err(|e| ReportError::Projection(e.to_string()))?;
        manifest_source = source;
        (path.as_path(), manifest_source.as_str())
    };
    let parsed = if options.manifest.is_some() {
        match crate::render::manifest::parse(original)
            .and_then(|manifest| crate::render::run(&manifest))
        {
            Ok(parsed) => parsed,
            Err(error) => {
                return Ok(Checked::ManifestError {
                    path: path.to_owned(),
                    sha256: crate::utf8_file::sha256(original),
                    error,
                });
            }
        }
    } else {
        ParsedSpec::parse(original)
    };
    let mut report = analyze(&parsed, options.policy, &options.defines);
    if options.upgrade {
        report.upgrade = Some(upgrade::query(
            original,
            options.repology_project.as_deref(),
        ));
    }
    if options.manifest.is_some() {
        report.set_manifest_input(original);
    }
    if let Some(input) = &spec_input {
        report.set_spec_revision(input.revision.as_deref());
    }
    if options.materials {
        // A committed SPEC is not a claim that the recipe working tree contains
        // its matching materials. Require the caller's prepared directory here.
        let directory = options.source_dir.as_deref().or_else(|| {
            spec_input.as_ref().map_or_else(
                || {
                    Some(
                        path.parent()
                            .filter(|path| !path.as_os_str().is_empty())
                            .unwrap_or_else(|| Path::new(".")),
                    )
                },
                |input| input.revision.is_none().then(|| input.directory()),
            )
        });
        let cache = options
            .source_dir
            .is_none()
            .then(|| spec_input.as_ref().and_then(|input| input.sources()))
            .flatten();
        let mut inventory =
            materials::analyze(directory, cache.as_deref(), &parsed, &options.defines);
        let unchanged = if let Some(input) = &spec_input {
            input.is_unchanged()
        } else {
            fs_err::canonicalize(path)
                .and_then(|path| crate::utf8_file::is_unchanged(&path, original))
        };
        match unchanged {
            Ok(true) => {}
            Ok(false) => {
                inventory.invalidate("input-changed", "recipe changed during inventory; retry");
            }
            Err(error) => inventory.invalidate("input-read", error),
        }
        report.materials = Some(inventory);
    }
    Ok(Checked::Spec {
        path: path.to_owned(),
        report: Box::new(report),
    })
}

fn write_report(
    report: &CheckReport,
    path: &Path,
    format: ReportFormat,
) -> Result<(), ReportError> {
    match format {
        ReportFormat::Human => {
            report
                .write_human(path, &mut crate::output_cli::stderr())
                .map_err(ReportError::Stderr)?;
            if let Some(materials) = &report.materials {
                materials
                    .write_human(&mut io::stdout().lock())
                    .map_err(ReportError::Stdout)?;
            }
        }
        ReportFormat::Toml => report
            .write_toml(path, &mut io::stdout().lock())
            .map_err(ReportError::Stdout)?,
    }
    Ok(())
}
