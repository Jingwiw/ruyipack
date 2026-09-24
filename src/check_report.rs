// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Human and machine reports for one static SPEC check.

use std::{
    borrow::Cow,
    io::{self, Write},
    path::Path,
};

use serde::Serialize;

use crate::{
    parser_diagnostic::{self, Diagnostic as ParserDiagnostic},
    source_location::SourceLocation,
};

/// Product policy level, independent of the analyzer that produced a finding.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    Allow,
    Warn,
    Deny,
}

const FORMAT_VERSION: u32 = 2;

/// A check left unfinished, independent of any confirmed failure.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum IncompleteReason {
    ParserError,
    UnresolvedLicense,
    UnresolvedBuildRequirements,
}

impl IncompleteReason {
    fn explanation(self) -> &'static str {
        match self {
            Self::ParserError => "the SPEC parser reported an error",
            Self::UnresolvedLicense => "license expressions require RPM evaluation",
            Self::UnresolvedBuildRequirements => "build requirements require RPM evaluation",
        }
    }
}

/// One selected static rule and its severity for confirmed violations.
#[derive(Serialize)]
pub(crate) struct SelectedRule {
    pub(crate) code: &'static str,
    pub(crate) severity: Severity,
}

enum CheckStatus {
    Pass,
    Fail,
    Incomplete,
}

/// Results produced by one execution of the static SPEC check.
pub(crate) struct CheckReport {
    sha256: String,
    status: CheckStatus,
    incomplete_reasons: Vec<IncompleteReason>,
    selected_rules: Vec<SelectedRule>,
    parser_diagnostics: Vec<ParserDiagnostic>,
    findings: Vec<Finding>,
}

impl CheckReport {
    /// Records a check stopped by parser errors.
    pub(crate) fn incomplete(
        source: &str,
        selected_rules: Vec<SelectedRule>,
        parser_diagnostics: Vec<ParserDiagnostic>,
    ) -> Self {
        Self {
            sha256: crate::utf8_file::digest(source),
            status: CheckStatus::Incomplete,
            incomplete_reasons: vec![IncompleteReason::ParserError],
            selected_rules,
            parser_diagnostics,
            findings: Vec::new(),
        }
    }

    /// Combines findings and records any unresolved field checks.
    pub(crate) fn analyzed(
        source: &str,
        selected_rules: Vec<SelectedRule>,
        parser_diagnostics: Vec<ParserDiagnostic>,
        mut findings: Vec<Finding>,
        mut incomplete_reasons: Vec<IncompleteReason>,
    ) -> Self {
        findings.sort_by(|left, right| {
            left.span
                .bytes
                .start
                .cmp(&right.span.bytes.start)
                .then_with(|| left.code.cmp(right.code))
                .then_with(|| left.message.cmp(&right.message))
        });
        incomplete_reasons.sort_unstable();
        incomplete_reasons.dedup();
        let status = if findings
            .iter()
            .any(|finding| finding.severity == Severity::Deny)
        {
            CheckStatus::Fail
        } else if !incomplete_reasons.is_empty() {
            CheckStatus::Incomplete
        } else {
            CheckStatus::Pass
        };
        Self {
            sha256: crate::utf8_file::digest(source),
            status,
            incomplete_reasons,
            selected_rules,
            parser_diagnostics,
            findings,
        }
    }

    /// Returns whether the selected static checks passed.
    pub(crate) fn is_success(&self) -> bool {
        matches!(self.status, CheckStatus::Pass)
    }

    /// Writes human-readable parser diagnostics and static-check findings.
    pub(crate) fn write_human(&self, path: &Path, writer: &mut impl Write) -> io::Result<()> {
        parser_diagnostic::write(path, &self.parser_diagnostics, writer)?;
        for finding in &self.findings {
            let severity = match finding.severity {
                Severity::Deny => "error",
                Severity::Warn => "warning",
                Severity::Allow => "diagnostic",
            };
            let span = &finding.span;
            writeln!(
                writer,
                "{}:{}:{}: {severity}[{}]: {}",
                path.display(),
                span.start.0,
                span.start.1,
                finding.code,
                finding.message
            )?;
        }
        for reason in &self.incomplete_reasons {
            writeln!(
                writer,
                "error: check incomplete because {}",
                reason.explanation()
            )?;
        }
        Ok(())
    }

    /// Borrows the structured report for embedding without a JSON round trip.
    pub(crate) fn structured<'a>(&'a self, path: &'a Path) -> impl Serialize + 'a {
        MachineReport {
            format_version: FORMAT_VERSION,
            input: InputIdentity {
                display_path: path.to_string_lossy(),
                sha256: &self.sha256,
            },
            evidence: Evidence {
                stage: "spec-static",
                status: self.status.name(),
                incomplete_reasons: &self.incomplete_reasons,
                tool: ToolIdentity {
                    name: "ruyipack",
                    version: env!("CARGO_PKG_VERSION"),
                },
                components: [
                    ComponentIdentity {
                        name: "rpm-spec",
                        version: env!("RUYIPACK_RPM_SPEC_VERSION"),
                        repository: env!("RUYIPACK_RPM_SPEC_REPOSITORY"),
                        revision: env!("RUYIPACK_RPM_SPEC_REVISION"),
                    },
                    ComponentIdentity {
                        name: "rpm-spec-analyzer",
                        version: env!("RUYIPACK_RPM_SPEC_ANALYZER_VERSION"),
                        repository: env!("RUYIPACK_RPM_SPEC_ANALYZER_REPOSITORY"),
                        revision: env!("RUYIPACK_RPM_SPEC_ANALYZER_REVISION"),
                    },
                ],
                selected_rules: &self.selected_rules,
            },
            parser_diagnostics: &self.parser_diagnostics,
            findings: &self.findings,
        }
    }

    /// Writes one deterministic JSON object.
    pub(crate) fn write_json(&self, path: &Path, writer: &mut impl Write) -> io::Result<()> {
        let report = self.structured(path);
        let json = serde_json::to_string(&report)
            .expect("the machine check report contains only JSON-compatible values");
        writeln!(writer, "{json}")
    }
}

impl CheckStatus {
    fn name(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Serialize)]
struct MachineReport<'a> {
    format_version: u32,
    input: InputIdentity<'a>,
    evidence: Evidence<'a>,
    parser_diagnostics: &'a [ParserDiagnostic],
    findings: &'a [Finding],
}

#[derive(Serialize)]
struct InputIdentity<'a> {
    display_path: Cow<'a, str>,
    sha256: &'a str,
}

#[derive(Serialize)]
struct Evidence<'a> {
    stage: &'static str,
    status: &'static str,
    incomplete_reasons: &'a [IncompleteReason],
    tool: ToolIdentity,
    components: [ComponentIdentity; 2],
    selected_rules: &'a [SelectedRule],
}

#[derive(Serialize)]
struct ToolIdentity {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct ComponentIdentity {
    name: &'static str,
    version: &'static str,
    repository: &'static str,
    revision: &'static str,
}

/// A finding with its actual producer, independent of the command that requested it.
#[derive(Serialize)]
pub(crate) struct Finding {
    pub(crate) producer: &'static str,
    pub(crate) code: &'static str,
    pub(crate) severity: Severity,
    pub(crate) message: String,
    pub(crate) span: SourceLocation,
}
