// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared static SPEC checks, rule selection, and the check command boundary.

pub(crate) mod build;
pub(crate) mod license;
pub(crate) mod materials;
pub(crate) mod metadata;

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::{
    check_report::{CheckReport, Finding, IncompleteReason, SelectedRule, Severity},
    output_cli::{ReportError, ReportFormat, report_input},
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
            (Self::Submit, "RPK005") => Severity::Deny,
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
#[command(group(clap::ArgGroup::new("check-input").args(["work", "spec", "manifest"]).required(true)))]
pub(crate) struct Options {
    #[command(flatten)]
    input: SpecOptions,
    /// Check an authoring manifest by rendering it in memory, without downloads or writes.
    #[arg(long, conflicts_with_all = ["work", "spec", "pkgname", "defines"],
        value_name = "PATH")]
    manifest: Option<PathBuf>,
    /// Also check staged Source/Patch files and report sizes and SHA-256 digests.
    #[arg(long)]
    materials: bool,
    /// Prepared RPM _sourcedir; defaults to the input directory, but must be explicit without a checkout.
    #[arg(long, requires = "materials", value_name = "DIR")]
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

pub(crate) fn run(options: &Options) -> Result<bool, ReportError> {
    let spec_input = if options.manifest.is_none() {
        let Some(input) = report_input(
            options.input.resolve(),
            &options.input.display(),
            options.format,
        )?
        else {
            return Ok(false);
        };
        Some(input)
    } else {
        None
    };
    let manifest_source;
    let (path, original) = if let Some(input) = &spec_input {
        (input.path.as_path(), input.source.as_str())
    } else {
        let path = options.manifest.as_ref().expect("SPEC or manifest input");
        let Some(source) = report_input(
            crate::utf8_file::read(path),
            &path.to_string_lossy(),
            options.format,
        )?
        else {
            return Ok(false);
        };
        manifest_source = source;
        (path.as_path(), manifest_source.as_str())
    };
    let parsed = if options.manifest.is_some() {
        match crate::render::manifest::parse(original)
            .and_then(|manifest| crate::render::run(&manifest))
        {
            Ok(parsed) => parsed,
            Err(error) => {
                if matches!(options.format, ReportFormat::Toml) {
                    crate::output_cli::write_failure(
                        crate::report::Input {
                            display_path: path.to_string_lossy(),
                            sha256: Some(&crate::utf8_file::sha256(original)),
                            revision: None,
                        },
                        crate::report::failure("invalid-manifest", &error),
                    )?;
                    return Ok(false);
                }
                return Err(ReportError::Manifest(error));
            }
        }
    } else {
        ParsedSpec::parse(original)
    };
    let mut report = analyze(&parsed, options.policy, &options.defines);
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
        let mut inventory = materials::analyze(directory, &parsed, &options.defines);
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
    write_report(&report, path, options.format)?;
    Ok(report.is_success())
}

fn write_report(
    report: &CheckReport,
    path: &Path,
    format: ReportFormat,
) -> Result<(), ReportError> {
    match format {
        ReportFormat::Human => {
            report
                .write_human(path, &mut io::stderr().lock())
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
