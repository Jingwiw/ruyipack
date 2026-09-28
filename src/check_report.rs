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
    UnresolvedSources,
}

impl IncompleteReason {
    fn explanation(self) -> &'static str {
        match self {
            Self::UnresolvedSources => "Source declarations require an unambiguous static context",
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
    policy: crate::check::Policy,
    source_uncertainty: Option<String>,
    defines: Vec<String>,
    status: CheckStatus,
    incomplete_reasons: Vec<IncompleteReason>,
    selected_rules: Vec<SelectedRule>,
    parser_diagnostics: Vec<ParserDiagnostic>,
    findings: Vec<Finding>,
}

impl CheckReport {
    /// Combines findings and records any unresolved field checks.
    pub(crate) fn analyzed(
        source: &str,
        selected_rules: Vec<SelectedRule>,
        parser_diagnostics: Vec<ParserDiagnostic>,
        result: crate::check::RuleResult,
        policy: crate::check::Policy,
        defines: &[String],
    ) -> Self {
        let crate::check::RuleResult {
            mut findings,
            mut incomplete_reasons,
            source_uncertainty,
        } = result;
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
            defines: defines.to_vec(),
            sha256: crate::utf8_file::sha256(source),
            policy,
            source_uncertainty,
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

    /// Positions move after edits. Compare rule facts and multiplicities, not offsets;
    /// this explains a blocked edit, never weakens its publication gate.
    pub(crate) fn introduced_static_blockers(&self, baseline: &Self) -> Option<bool> {
        let new = self
            .incomplete_reasons
            .iter()
            .any(|reason| !baseline.incomplete_reasons.contains(reason))
            || self
                .findings
                .iter()
                .filter(|finding| finding.severity == Severity::Deny)
                .any(|finding| {
                    let same = |other: &&Finding| {
                        other.severity == Severity::Deny
                            && other.code == finding.code
                            && other.producer == finding.producer
                            && other.message == finding.message
                    };
                    self.findings.iter().filter(same).count()
                        > baseline.findings.iter().filter(same).count()
                });
        // Equal incomplete categories do not prove the unresolved facts are equal.
        if new {
            Some(true)
        } else if self.incomplete_reasons.is_empty() {
            Some(false)
        } else {
            None
        }
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
                "{}: error: check incomplete because {}",
                path.display(),
                if *reason == IncompleteReason::UnresolvedSources {
                    self.source_uncertainty
                        .as_deref()
                        .unwrap_or(reason.explanation())
                } else {
                    reason.explanation()
                }
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
                policy: self.policy,
                defines: &self.defines,
                source_uncertainty: self.source_uncertainty.as_deref(),
                not_checked: ["source-content", "native-rpm", "build"],
                status: self.status.name(),
                incomplete_reasons: &self.incomplete_reasons,
                tool: crate::tool::identity(),
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
        serde_json::to_writer(&mut *writer, &self.structured(path))?;
        writeln!(writer)
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
    policy: crate::check::Policy,
    source_uncertainty: Option<&'a str>,
    defines: &'a [String],
    not_checked: [&'static str; 3],
    stage: &'static str,
    status: &'static str,
    incomplete_reasons: &'a [IncompleteReason],
    tool: crate::tool::Identity,
    components: [ComponentIdentity; 2],
    selected_rules: &'a [SelectedRule],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_preserves_writer_error_kind() {
        struct BrokenWriter;
        impl Write for BrokenWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let report = crate::check::analyze(
            &crate::spec::ParsedSpec::parse(""),
            crate::check::Policy::Authoring,
            &[],
        );
        let error = report
            .write_json(Path::new("input.spec"), &mut BrokenWriter)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
