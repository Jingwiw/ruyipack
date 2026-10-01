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

use serde::Serialize;

use crate::{
    output_cli::{self, HumanLevel},
    parser_diagnostic::{self, Diagnostic as ParserDiagnostic},
    source_location::SourceLocation,
};

/// Product policy level, independent of the analyzer that produced a finding.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    Warn,
    Deny,
}

impl Severity {
    pub(crate) fn human_level(self) -> HumanLevel {
        match self {
            Self::Deny => HumanLevel::Error,
            Self::Warn => HumanLevel::Warn,
        }
    }
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

/// Results produced by one execution of the static SPEC check.
pub(crate) struct CheckReport {
    sha256: String,
    source_revision: Option<String>,
    generated_spec_sha256: Option<String>,
    pub(crate) materials: Option<crate::check::materials::Report>,
    policy: crate::check::Policy,
    source_uncertainty: Option<String>,
    defines: Vec<String>,
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
        Self {
            generated_spec_sha256: None,
            source_revision: None,
            materials: None,
            defines: defines.to_vec(),
            sha256: crate::utf8_file::sha256(source),
            policy,
            source_uncertainty,
            incomplete_reasons,
            selected_rules,
            parser_diagnostics,
            findings,
        }
    }

    fn status(&self) -> &'static str {
        if self
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Deny)
        {
            "fail"
        } else if !self.incomplete_reasons.is_empty() {
            "incomplete"
        } else {
            "pass"
        }
    }

    /// Returns whether the selected static checks passed.
    pub(crate) fn is_success(&self) -> bool {
        self.status() == "pass" && self.materials.as_ref().is_none_or(|report| report.valid)
    }

    pub(crate) fn findings(&self) -> &[Finding] {
        &self.findings
    }

    pub(crate) fn diagnostics(&self) -> &[ParserDiagnostic] {
        &self.parser_diagnostics
    }

    /// Authoring may retain a violation only when its complete rule inputs are
    /// unchanged. Equal messages or rule IDs are not proof of an unchanged fact.
    /// This does not turn the candidate into a successful whole-package check.
    pub(crate) fn allows_edit(&self, baseline: &Self) -> bool {
        if self.is_success() {
            return true;
        }
        self.policy == crate::check::Policy::Authoring
            && baseline.policy == self.policy
            && self.defines == baseline.defines
            && self.incomplete_reasons.is_empty()
            && baseline.incomplete_reasons.is_empty()
            && self.materials.as_ref().is_none_or(|report| report.valid)
            && self
                .changes(baseline)
                .filter(|(finding, _)| finding.severity == Severity::Deny)
                .all(|(finding, retained)| finding.rule_inputs.is_some() && retained)
    }

    /// Match occurrences, not just rule IDs: one baseline issue cannot excuse
    /// two candidate issues. Unknown inputs are display matches only.
    pub(crate) fn changes<'a>(
        &'a self,
        baseline: &'a Self,
    ) -> impl Iterator<Item = (&'a Finding, bool)> {
        let mut remaining = baseline.findings.iter().collect::<Vec<_>>();
        self.findings.iter().map(move |finding| {
            let matched = remaining.iter().position(|other| finding.same_issue(other));
            if let Some(index) = matched {
                remaining.swap_remove(index);
            }
            (finding, matched.is_some())
        })
    }

    /// Identify immutable recipe content read without creating a checkout.
    pub(crate) fn set_spec_revision(&mut self, revision: Option<&str>) {
        self.source_revision = revision.map(str::to_owned);
    }

    /// Keep manifest bytes as the input identity; positions still refer to generated SPEC.
    pub(crate) fn set_manifest_input(&mut self, original: &str) {
        self.generated_spec_sha256 = Some(std::mem::replace(
            &mut self.sha256,
            crate::utf8_file::sha256(original),
        ));
    }

    /// Positions move after edits. Compare supplied rule inputs and multiplicities,
    /// not offsets; unknown rule inputs remain explanatory, not admission evidence.
    pub(crate) fn introduced_static_blockers(&self, baseline: &Self) -> Option<bool> {
        let new = self
            .incomplete_reasons
            .iter()
            .any(|reason| !baseline.incomplete_reasons.contains(reason))
            || self
                .changes(baseline)
                .any(|(finding, retained)| finding.severity == Severity::Deny && !retained);
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
    pub(crate) fn write_human(
        &self,
        path: &Path,
        writer: &mut output_cli::HumanOutput<impl Write>,
    ) -> io::Result<()> {
        if !self.parser_diagnostics.is_empty()
            || !self.findings.is_empty()
            || !self.incomplete_reasons.is_empty()
        {
            writer.message(
                HumanLevel::Info,
                Some(path),
                format_args!(
                    "{}",
                    if self.generated_spec_sha256.is_some() {
                        "checking generated SPEC; diagnostic positions refer to that SPEC, not TOML"
                    } else {
                        "checking SPEC"
                    }
                ),
            )?;
        }
        parser_diagnostic::write(&self.parser_diagnostics, writer)?;
        for finding in &self.findings {
            finding.write_human(writer)?;
        }
        self.write_incomplete(writer)
    }

    pub(crate) fn write_incomplete(
        &self,
        writer: &mut output_cli::HumanOutput<impl Write>,
    ) -> io::Result<()> {
        for reason in &self.incomplete_reasons {
            writer.message(
                HumanLevel::Error,
                None,
                format_args!(
                    "check incomplete because {}",
                    if *reason == IncompleteReason::UnresolvedSources {
                        self.source_uncertainty
                            .as_deref()
                            .unwrap_or(reason.explanation())
                    } else {
                        reason.explanation()
                    }
                ),
            )?;
        }
        Ok(())
    }

    /// Borrows the structured report for direct embedding in command results.
    pub(crate) fn structured<'a>(&'a self, path: &'a Path) -> Report<'a> {
        Report {
            format_version: FORMAT_VERSION,
            valid: self.is_success(),
            generated_spec_sha256: self.generated_spec_sha256.as_deref(),
            materials: self.materials.as_ref(),
            input: crate::report::Input {
                display_path: path.to_string_lossy(),
                sha256: Some(&self.sha256),
                revision: self.source_revision.as_deref(),
            },
            evidence: Evidence {
                stage: "spec-static",
                policy: self.policy,
                defines: &self.defines,
                source_uncertainty: self.source_uncertainty.as_deref(),
                not_checked: ["source-content", "native-rpm", "build"],
                status: self.status(),
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

    /// Writes one deterministic TOML report.
    pub(crate) fn write_toml(&self, path: &Path, writer: &mut impl Write) -> io::Result<()> {
        crate::report::write(writer, &self.structured(path))
    }
}

#[derive(Serialize)]
pub(crate) struct Report<'a> {
    valid: bool,
    generated_spec_sha256: Option<&'a str>,
    materials: Option<&'a crate::check::materials::Report>,
    format_version: u32,
    input: crate::report::Input<'a>,
    evidence: Evidence<'a>,
    parser_diagnostics: &'a [ParserDiagnostic],
    findings: &'a [Finding],
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
    /// All inputs used by this violation's rule, including its field identity.
    /// Producers without this evidence cannot authorize retaining a failed rule.
    #[serde(skip)]
    pub(crate) rule_inputs: Option<Vec<String>>,
}

impl Finding {
    /// Unknown inputs can share a display identity, never admission evidence.
    fn same_issue(&self, other: &Self) -> bool {
        self.severity == other.severity
            && self.producer == other.producer
            && self.code == other.code
            && match (&self.rule_inputs, &other.rule_inputs) {
                (Some(left), Some(right)) => left == right,
                (None, None) => self.message == other.message,
                _ => false,
            }
    }

    pub(crate) fn write_human(
        &self,
        writer: &mut output_cli::HumanOutput<impl Write>,
    ) -> io::Result<()> {
        writer.diagnostic(
            self.severity.human_level(),
            Some(self.span.start),
            Some(self.code),
            format_args!("{}", self.message),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = include_str!("../tests/fixtures/ed.spec");

    fn check(source: &str) -> CheckReport {
        crate::check::analyze(
            &crate::spec::ParsedSpec::parse(source),
            crate::check::Policy::Authoring,
            &[],
        )
    }

    #[test]
    fn edit_retains_only_proven_unchanged_rule_inputs_not_a_successful_check() {
        let source = SPEC.replace("BuildRequires:  autoconf\n", "");
        let baseline = check(&source);
        let changed = source.replace("1.22.5", "1.22.600");
        let candidate = check(&changed);
        assert!(!candidate.is_success());
        assert!(candidate.allows_edit(&baseline));
        assert_eq!(candidate.introduced_static_blockers(&baseline), Some(false));

        // Even an unrelated direct requirement is an input to this rule; equal
        // missing-tool messages are not enough to admit a changed contract.
        let other_requirements =
            check(&changed.replace("BuildRequires:  lzip", "BuildRequires:  zip"));
        assert!(!other_requirements.allows_edit(&baseline));
        assert_eq!(
            other_requirements.introduced_static_blockers(&baseline),
            Some(true)
        );

        let submit = crate::check::analyze(
            &crate::spec::ParsedSpec::parse(&changed),
            crate::check::Policy::Submit,
            &[],
        );
        assert!(!submit.is_success());
        assert!(!submit.allows_edit(&baseline));
    }

    #[test]
    fn equal_error_text_cannot_hide_a_changed_invalid_value_or_reuse_one_finding() {
        let url = "https://www.gnu.org/software/ed/";
        let source = SPEC.replace(url, "ftp://example.org/old");
        let baseline = check(&source);
        let retained = check(&source.replace("1.22.5", "1.22.6"));
        assert!(retained.allows_edit(&baseline));

        let changed = check(&source.replace("/old", "/new"));
        assert_eq!(baseline.findings[0].message, changed.findings[0].message);
        assert!(!changed.allows_edit(&baseline));
        assert_eq!(changed.introduced_static_blockers(&baseline), Some(true));

        let duplicate = check(&source.replace(
            "URL:            ftp://example.org/old",
            "URL:            ftp://example.org/old\nURL:            ftp://example.org/old",
        ));
        assert!(!duplicate.allows_edit(&baseline));
    }

    #[test]
    fn unresolved_or_unproved_rule_contexts_never_authorize_retaining_blockers() {
        for source in [
            SPEC.replace("Summary:        A line-oriented text editor\n", ""),
            SPEC.replace(
                "License:        GPL-3.0-or-later AND LGPL-2.1-or-later",
                "License: %{upstream_license}",
            ),
            format!("%if 1\n{SPEC}"),
            SPEC.replace(
                "URL:            https://www.gnu.org/software/ed/",
                "%if 1\nURL: ftp://example.org/old\n%endif",
            ),
            format!("{SPEC}\n%package docs\nSummary: Docs\nURL: ftp://example.org/old\n"),
        ] {
            let baseline = check(&source);
            let candidate = check(&source.replace("1.22.5", "1.22.6"));
            assert!(!candidate.is_success(), "{source}");
            assert!(!candidate.allows_edit(&baseline), "{source}");
        }
    }

    #[test]
    fn toml_preserves_writer_error_kind() {
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
            .write_toml(Path::new("input.spec"), &mut BrokenWriter)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);

        let error = report
            .write_human(
                Path::new("input.spec"),
                &mut output_cli::HumanOutput::new(BrokenWriter, false),
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn shortened_human_paths_do_not_change_machine_identity() {
        let path = std::env::current_dir().unwrap().join("pkg.spec");
        let mut report = check("");
        let mut human = Vec::new();
        report
            .write_human(&path, &mut output_cli::HumanOutput::new(&mut human, false))
            .unwrap();
        let human = String::from_utf8(human).unwrap();
        assert!(human.contains("[ERROR]"), "{human:?}");
        assert!(
            human.contains("[INFO] pkg.spec: checking SPEC"),
            "{human:?}"
        );
        assert_eq!(human.matches("pkg.spec").count(), 1, "{human:?}");
        assert!(human.contains(" spec[1:1] [RPM010]:"), "{human:?}");
        assert!(!human.contains(path.to_str().unwrap()), "{human:?}");
        let mut machine = Vec::new();
        report.write_toml(&path, &mut machine).unwrap();
        let machine_report: toml::Value =
            toml::from_str(std::str::from_utf8(&machine).unwrap()).unwrap();
        assert_eq!(
            machine_report["input"]["display_path"].as_str(),
            Some(path.to_string_lossy().as_ref())
        );
        assert_eq!(
            machine_report["findings"][0]["severity"].as_str(),
            Some("deny")
        );

        let manifest_path = std::env::current_dir().unwrap().join("pkg.toml");
        report.set_manifest_input("[package]\n");
        let mut human = Vec::new();
        report
            .write_human(
                &manifest_path,
                &mut output_cli::HumanOutput::new(&mut human, false),
            )
            .unwrap();
        let human = String::from_utf8(human).unwrap();
        assert!(
            human.contains("pkg.toml: checking generated SPEC"),
            "{human}"
        );
        assert!(human.contains("spec[1:1] [RPM010]:"), "{human}");
        assert!(!human.contains("pkg.toml:1:1"), "{human}");
        machine.clear();
        report.write_toml(&manifest_path, &mut machine).unwrap();
        let machine: toml::Value = toml::from_str(std::str::from_utf8(&machine).unwrap()).unwrap();
        assert_eq!(
            machine["input"]["display_path"].as_str(),
            Some(manifest_path.to_string_lossy().as_ref())
        );
        assert_eq!(
            machine["findings"][0]["span"]["start_line"].as_integer(),
            Some(1)
        );
    }
}
