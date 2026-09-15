// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Human and machine reports for one static SPEC check.

use std::{
    io::{self, Write},
    path::Path,
};

use rpm_spec::{
    ast::Span,
    parse_result::{Diagnostic as ParserDiagnostic, Severity as ParserSeverity},
};
use rpm_spec_analyzer::diagnostic::{Diagnostic, Severity};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::parser_diagnostic;

const FORMAT_VERSION: u32 = 1;
const INCOMPLETE_PARSER_ERROR: &str = "parser-error";
const RPM_SPEC_REPOSITORY: &str = "https://github.com/openRuyi-Project/rpm-spec";
const RPM_SPEC_TOOL_REPOSITORY: &str = "https://github.com/openRuyi-Project/rpm-spec-tool";

/// One analyzer rule selected by the current check policy.
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
    selected_rules: Vec<SelectedRule>,
    parser_diagnostics: Vec<ParserDiagnostic>,
    findings: Vec<Diagnostic>,
}

impl CheckReport {
    /// Records a check stopped by parser errors.
    pub(crate) fn incomplete(
        source: &str,
        selected_rules: Vec<SelectedRule>,
        parser_diagnostics: Vec<ParserDiagnostic>,
    ) -> Self {
        Self {
            sha256: source_digest(source),
            status: CheckStatus::Incomplete,
            selected_rules,
            parser_diagnostics,
            findings: Vec::new(),
        }
    }

    /// Records a completed analyzer run.
    pub(crate) fn analyzed(
        source: &str,
        selected_rules: Vec<SelectedRule>,
        parser_diagnostics: Vec<ParserDiagnostic>,
        mut findings: Vec<Diagnostic>,
    ) -> Self {
        findings.sort_by(|left, right| {
            left.primary_span
                .start_byte
                .cmp(&right.primary_span.start_byte)
                .then_with(|| left.lint_id.cmp(right.lint_id))
                .then_with(|| left.message.cmp(&right.message))
        });
        let status = if findings
            .iter()
            .any(|finding| finding.severity == Severity::Deny)
        {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        };
        Self {
            sha256: source_digest(source),
            status,
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
        parser_diagnostic::write(&self.parser_diagnostics, writer)?;
        for finding in &self.findings {
            let severity = match finding.severity {
                Severity::Deny => "error",
                Severity::Warn => "warning",
                Severity::Allow => "diagnostic",
            };
            let span = finding.primary_span;
            writeln!(
                writer,
                "{}:{}:{}: {severity}[{}]: {}",
                path.display(),
                span.start_line,
                span.start_column,
                finding.lint_id,
                finding.message
            )?;
        }
        if matches!(self.status, CheckStatus::Incomplete) {
            writeln!(
                writer,
                "error: check incomplete because the SPEC parser reported an error"
            )?;
        }
        Ok(())
    }

    /// Writes one deterministic JSON object.
    pub(crate) fn write_json(&self, path: &Path, writer: &mut impl Write) -> io::Result<()> {
        let path = path.to_string_lossy();
        let report = MachineReport {
            format_version: FORMAT_VERSION,
            input: InputIdentity {
                display_path: &path,
                sha256: &self.sha256,
            },
            evidence: Evidence {
                stage: "spec-static",
                status: self.status.name(),
                reason: self.status.reason(),
                tool: ToolIdentity {
                    name: "ruyipack",
                    version: env!("CARGO_PKG_VERSION"),
                },
                components: [
                    ComponentIdentity {
                        name: "rpm-spec",
                        version: env!("RUYIPACK_RPM_SPEC_VERSION"),
                        repository: RPM_SPEC_REPOSITORY,
                        revision: env!("RUYIPACK_RPM_SPEC_REVISION"),
                    },
                    ComponentIdentity {
                        name: "rpm-spec-analyzer",
                        version: env!("RUYIPACK_RPM_SPEC_ANALYZER_VERSION"),
                        repository: RPM_SPEC_TOOL_REPOSITORY,
                        revision: env!("RUYIPACK_RPM_SPEC_ANALYZER_REVISION"),
                    },
                ],
                selected_rules: self
                    .selected_rules
                    .iter()
                    .map(SelectedRuleRecord::from)
                    .collect(),
            },
            parser_diagnostics: self
                .parser_diagnostics
                .iter()
                .map(ParserDiagnosticRecord::from)
                .collect(),
            findings: self.findings.iter().map(Finding::from).collect(),
        };
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

    fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Incomplete => Some(INCOMPLETE_PARSER_ERROR),
            Self::Pass | Self::Fail => None,
        }
    }
}

fn source_digest(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

#[derive(Serialize)]
struct MachineReport<'a> {
    format_version: u32,
    input: InputIdentity<'a>,
    evidence: Evidence<'a>,
    parser_diagnostics: Vec<ParserDiagnosticRecord<'a>>,
    findings: Vec<Finding<'a>>,
}

#[derive(Serialize)]
struct InputIdentity<'a> {
    display_path: &'a str,
    sha256: &'a str,
}

#[derive(Serialize)]
struct Evidence<'a> {
    stage: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    tool: ToolIdentity,
    components: [ComponentIdentity; 2],
    selected_rules: Vec<SelectedRuleRecord<'a>>,
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

#[derive(Serialize)]
struct SelectedRuleRecord<'a> {
    code: &'a str,
    severity: &'static str,
}

impl<'a> From<&'a SelectedRule> for SelectedRuleRecord<'a> {
    fn from(rule: &'a SelectedRule) -> Self {
        Self {
            code: rule.code,
            severity: analyzer_severity(rule.severity),
        }
    }
}

#[derive(Serialize)]
struct ParserDiagnosticRecord<'a> {
    severity: &'static str,
    code: Option<&'a str>,
    span: Option<SourceSpan>,
    message: &'a str,
    notes: &'a [String],
}

impl<'a> From<&'a ParserDiagnostic> for ParserDiagnosticRecord<'a> {
    fn from(diagnostic: &'a ParserDiagnostic) -> Self {
        Self {
            severity: parser_severity(diagnostic.severity),
            code: diagnostic.code.as_deref(),
            span: diagnostic.span.map(SourceSpan::from),
            message: &diagnostic.message,
            notes: &diagnostic.notes,
        }
    }
}

#[derive(Serialize)]
struct Finding<'a> {
    producer: &'static str,
    code: &'a str,
    severity: &'static str,
    message: &'a str,
    span: SourceSpan,
}

impl<'a> From<&'a Diagnostic> for Finding<'a> {
    fn from(diagnostic: &'a Diagnostic) -> Self {
        Self {
            producer: "rpm-spec-analyzer",
            code: diagnostic.lint_id,
            severity: analyzer_severity(diagnostic.severity),
            message: &diagnostic.message,
            span: SourceSpan::from(diagnostic.primary_span),
        }
    }
}

#[derive(Serialize)]
struct SourceSpan {
    start_byte: usize,
    end_byte: usize,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
}

impl From<Span> for SourceSpan {
    fn from(span: Span) -> Self {
        Self {
            start_byte: span.start_byte,
            end_byte: span.end_byte,
            start_line: span.start_line,
            start_column: span.start_column,
            end_line: span.end_line,
            end_column: span.end_column,
        }
    }
}

fn analyzer_severity(severity: Severity) -> &'static str {
    match severity {
        Severity::Allow => "allow",
        Severity::Warn => "warn",
        Severity::Deny => "deny",
    }
}

fn parser_severity(severity: ParserSeverity) -> &'static str {
    match severity {
        ParserSeverity::Warning => "warning",
        ParserSeverity::Error => "error",
        _ => "unknown",
    }
}
