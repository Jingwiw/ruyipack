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
    io::{self, Write},
    path::PathBuf,
};

use crate::{
    check_report::{CheckReport, Finding, IncompleteReason, SelectedRule, Severity},
    output_cli::{ReportError, ReportFormat, read_source},
    parser_diagnostic,
    spec::ParsedSpec,
};

/// Findings and unfinished checks are independent facts, not severity conventions.
#[derive(Default)]
pub(crate) struct RuleResult {
    pub(crate) source_uncertainty: Option<String>,
    pub(crate) findings: Vec<Finding>,
    pub(crate) incomplete_reasons: Vec<IncompleteReason>,
}

// Required main-package tags for this profile.
const REQUIRED_TAG_LINT_IDS: [&str; 6] =
    ["RPM010", "RPM011", "RPM012", "RPM013", "RPM014", "RPM015"];

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
    let mut selected_rules: Vec<_> = REQUIRED_TAG_LINT_IDS
        .iter()
        .map(|&code| SelectedRule {
            code,
            severity: Severity::Deny,
        })
        .collect();
    let diagnostics = spec.diagnostics();

    let parser_error = diagnostics
        .iter()
        .any(|item| item.severity == parser_diagnostic::Severity::Error);
    let mut findings = if parser_error {
        Vec::new()
    } else {
        spec.analyzer_findings(&selected_rules)
    };
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
pub(crate) struct Options {
    /// RPM SPEC to check.
    #[arg(value_name = "SPEC", required_unless_present = "manifest")]
    spec: Option<PathBuf>,
    /// Check an authoring manifest by rendering it in memory, without downloads or writes.
    #[arg(long, conflicts_with_all = ["spec", "defines"], value_name = "PATH")]
    manifest: Option<PathBuf>,
    /// Also check staged Source/Patch files and report sizes and SHA-256 digests.
    #[arg(long)]
    materials: bool,
    /// Prepared RPM _sourcedir; defaults to the recipe directory, not the current directory.
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
    let path = options
        .manifest
        .as_ref()
        .or(options.spec.as_ref())
        .expect("required CLI input");
    let Some(original) = read_source(path, options.format)? else {
        return Ok(false);
    };
    let rendered;
    let mut authoring_report = None;
    let source = if options.manifest.is_some() {
        match crate::render::manifest::parse(&original)
            .and_then(|manifest| crate::render::run(&manifest))
        {
            Ok(result) => {
                authoring_report = Some(result.report);
                rendered = result.contents;
                &rendered
            }
            Err(error) => {
                if matches!(options.format, ReportFormat::Json) {
                    let mut stdout = io::stdout().lock();
                    let report = serde_json::json!({"format_version": 2, "valid": false,
                        "tool": crate::tool::identity(), "input": {"display_path":path,"sha256":crate::utf8_file::sha256(&original)},
                        "error": crate::output_cli::failure("invalid-manifest", &error)});
                    serde_json::to_writer(&mut stdout, &report)
                        .map_err(|e| ReportError::Stdout(e.into()))?;
                    writeln!(stdout).map_err(ReportError::Stdout)?;
                    return Ok(false);
                }
                return Err(ReportError::Manifest(error));
            }
        }
    } else {
        &original
    };
    let parsed = std::cell::LazyCell::new(|| ParsedSpec::parse(source));
    let mut report = match (options.policy, authoring_report) {
        (Policy::Authoring, Some(report)) => report,
        _ => analyze(&parsed, options.policy, &options.defines),
    };
    if options.manifest.is_some() {
        report.set_manifest_input(&original);
    }
    if options.materials {
        report.materials = Some(materials::analyze(
            path,
            &original,
            &parsed,
            &options.defines,
            options.source_dir.as_deref(),
        ));
    }
    match options.format {
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
        ReportFormat::Json => report
            .write_json(path, &mut io::stdout().lock())
            .map_err(ReportError::Stdout)?,
    }
    Ok(report.is_success())
}
