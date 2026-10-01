// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Human edit reports show changes; complete reports remain in TOML and check.

use crate::{
    check_report::{CheckReport, Severity},
    output_cli::{self, HumanLevel},
    parser_diagnostic,
};
use serde::Serialize;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{self, Write},
    path::Path,
};

#[derive(Serialize)]
pub(super) struct Envelope<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    scope: &'static str,
    operation: &'static str,
    success: bool,
    valid: Option<bool>,
    files: Vec<File<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    outcomes: Vec<Outcome<'a>>,
    error: Option<&'a super::EditError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    written: Vec<Cow<'a, str>>,
}

#[derive(Serialize)]
struct File<'a> {
    source: Cow<'a, str>,
    draft: Option<Cow<'a, str>>,
    stage: Cow<'a, str>,
    original_sha256: String,
    selected_fields: &'a [String],
    #[serde(flatten)]
    state: State<'a>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum State<'a> {
    PendingEdit { error: Option<&'a super::EditError> },
    Candidate(Box<CandidateState<'a>>),
}

#[derive(Serialize)]
struct CandidateState<'a> {
    changed: bool,
    profile: crate::profile::Identity,
    review_triggers: &'a [String],
    review_required: &'static [&'static str],
    source_hashes: Option<&'a crate::source::SourceHashes>,
    #[serde(flatten)]
    checked: Option<Checked<'a>>,
    #[serde(flatten)]
    cached: Option<Cached<'a>>,
}

#[derive(Serialize)]
struct Checked<'a> {
    valid: bool,
    admissible: bool,
    baseline_report: crate::check_report::Report<'a>,
    report: crate::check_report::Report<'a>,
    introduced_static_blockers: Option<bool>,
    error: Option<super::EditError>,
}

#[derive(Serialize)]
struct Cached<'a> {
    candidate_spec: Cow<'a, str>,
    diff: Option<&'a str>,
    diff_file: Option<Cow<'a, str>>,
}

#[derive(Serialize)]
struct Outcome<'a> {
    status: &'static str,
    path: Cow<'a, str>,
    sha256: Option<String>,
}

impl<'a> File<'a> {
    fn from_edit(item: &'a super::Edit) -> Self {
        let state = match item.candidate.as_ref() {
            Some(Ok(candidate)) => State::Candidate(Box::new(CandidateState {
                changed: candidate.spec.source() != item.snapshot.source(),
                profile: crate::profile::identity(),
                review_triggers: &candidate.review_triggers,
                review_required: if candidate.review_triggers.is_empty() {
                    &[]
                } else {
                    &[
                        "source-content-and-digests",
                        "patch-applicability",
                        "native-build",
                    ]
                },
                source_hashes: item.source_hashes.as_ref(),
                checked: item.baseline.as_ref().zip(candidate.report.as_ref()).map(
                    |(baseline, report)| Checked {
                        valid: report.is_success(),
                        admissible: report.allows_edit(baseline),
                        baseline_report: baseline.structured(&item.path),
                        report: report.structured(&item.path),
                        introduced_static_blockers: report.introduced_static_blockers(baseline),
                        error: super::static_check_error(item, candidate),
                    },
                ),
                cached: item.cache.as_ref().map(|cache| Cached {
                    candidate_spec: cache.path.to_string_lossy(),
                    diff: cache.diff.as_ref().map(|(_, text)| text.as_str()),
                    diff_file: cache.diff.as_ref().map(|(path, _)| path.to_string_lossy()),
                }),
            })),
            check => State::PendingEdit {
                error: check.and_then(|check| check.as_ref().err()),
            },
        };
        Self {
            source: item.path.to_string_lossy(),
            draft: item.draft.as_ref().map(|path| path.to_string_lossy()),
            stage: item.stage_dir.to_string_lossy(),
            original_sha256: crate::utf8_file::sha256(item.snapshot.source()),
            selected_fields: item.snapshot.selection(),
            state,
        }
    }
}

pub(super) fn envelope<'a>(
    result: &'a Result<super::EditResult, super::EditError>,
    options: &super::Options,
) -> Envelope<'a> {
    let files = result.as_ref().ok().map_or_else(Vec::new, |result| {
        result.inputs.iter().map(File::from_edit).collect()
    });
    let error = match result {
        Ok(result) => result.publication.as_ref().err(),
        Err(error) => Some(error),
    };
    let outcomes = result
        .as_ref()
        .ok()
        .and_then(|result| result.publication.as_ref().ok())
        .map_or_else(Vec::new, |outcomes| {
            outcomes
                .iter()
                .map(|outcome| {
                    use crate::file_output::EditOutcome;
                    let (status, path) = match outcome {
                        EditOutcome::Written(path) => ("written", path),
                        EditOutcome::Unchanged(path) => ("unchanged", path),
                        EditOutcome::Skipped(path) => ("skipped", path),
                    };
                    Outcome {
                        status,
                        path: path.to_string_lossy(),
                        sha256: (status != "skipped")
                            .then(|| {
                                crate::utf8_file::read(path)
                                    .ok()
                                    .map(|text| crate::utf8_file::sha256(&text))
                            })
                            .flatten(),
                    }
                })
                .collect()
        });
    Envelope {
        format_version: 4,
        tool: crate::tool::identity(),
        scope: "edit-stage",
        operation: if options.apply {
            "apply"
        } else if options.check {
            "check"
        } else if options.diff {
            "diff"
        } else {
            "stage"
        },
        success: result.as_ref().is_ok_and(super::EditResult::is_success),
        valid: result
            .as_ref()
            .ok()
            .and_then(super::EditResult::static_valid),
        files,
        outcomes,
        error,
        written: error
            .map_or(&[][..], super::EditError::written_paths)
            .iter()
            .map(|path| path.to_string_lossy())
            .collect(),
    }
}

pub(super) fn write_changes(
    path: &Path,
    baseline: &CheckReport,
    candidate: &CheckReport,
    writer: &mut output_cli::HumanOutput<impl Write>,
) -> io::Result<()> {
    if baseline.findings().is_empty()
        && candidate.findings().is_empty()
        && baseline.diagnostics().is_empty()
        && candidate.diagnostics().is_empty()
        && candidate.is_success()
    {
        return Ok(());
    }
    let changes = candidate.changes(baseline).collect::<Vec<_>>();
    let count = |report: &CheckReport| {
        report
            .findings()
            .iter()
            .filter(|f| f.severity == Severity::Deny)
            .count()
    };
    let inherited = changes
        .iter()
        .filter(|(finding, retained)| finding.severity == Severity::Deny && *retained)
        .count();
    writer.message(
        if count(candidate) > inherited {
            HumanLevel::Error
        } else {
            HumanLevel::Info
        },
        Some(path),
        format_args!(
            "candidate static blockers: new {}, inherited {}, resolved {}",
            count(candidate) - inherited,
            inherited,
            count(baseline) - inherited
        ),
    )?;
    let mut legacy = BTreeMap::new();
    for (finding, retained) in &changes {
        if *retained {
            legacy
                .entry((finding.code, finding.span.start))
                .or_insert((0usize, finding.severity.human_level()))
                .0 += 1;
            // Unknown rule inputs cannot authorize retaining a blocker. Keep
            // its concrete cause visible even when the issue is inherited.
            if finding.severity == Severity::Deny && finding.rule_inputs.is_none() {
                finding.write_human(writer)?;
            }
        } else {
            finding.write_human(writer)?;
        }
    }
    let retained_parser =
        write_parser_changes(baseline.diagnostics(), candidate.diagnostics(), writer)?;
    let warning_count = |report: &CheckReport| {
        report
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.severity == parser_diagnostic::Severity::Warning)
            .count()
            + report
                .findings()
                .iter()
                .filter(|finding| finding.severity == Severity::Warn)
                .count()
    };
    let inherited_warnings = retained_parser
        + changes
            .iter()
            .filter(|(finding, retained)| finding.severity == Severity::Warn && *retained)
            .count();
    if warning_count(baseline) + warning_count(candidate) != 0 {
        writer.message(
            HumanLevel::Warn,
            None,
            format_args!(
                "warnings: new {}, inherited {}, resolved {}",
                warning_count(candidate) - inherited_warnings,
                inherited_warnings,
                warning_count(baseline) - inherited_warnings
            ),
        )?;
    }
    for ((code, start), (count, level)) in legacy {
        writer.diagnostic(
            level,
            Some(start),
            Some(code),
            format_args!(
                "inherited {count} issue(s){}",
                match code {
                    "RPK004" => " in the declared BuildSystem contract",
                    "RPK005" => " in Source digests",
                    _ => "",
                }
            ),
        )?;
    }
    write_guidance(candidate, writer)
}

fn write_guidance(
    candidate: &CheckReport,
    writer: &mut output_cli::HumanOutput<impl Write>,
) -> io::Result<()> {
    candidate.write_incomplete(writer)?;
    if candidate
        .findings()
        .iter()
        .any(|finding| finding.code == "RPK005")
    {
        writer.message(
            HumanLevel::Info,
            None,
            format_args!(
                "Source digests: use edit --hash-source N to fill a digest; source verify compares downloaded bytes without changing declarations."
            ),
        )?;
    }
    if !candidate.is_success() {
        writer.message(
            HumanLevel::Info,
            None,
            format_args!(
                "Full candidate evidence: use edit --check --format toml with the same inputs; check reports the current SPEC."
            ),
        )?;
    }
    Ok(())
}

fn write_parser_changes(
    baseline: &[parser_diagnostic::Diagnostic],
    candidate: &[parser_diagnostic::Diagnostic],
    writer: &mut output_cli::HumanOutput<impl Write>,
) -> io::Result<usize> {
    // Locations can shift after a replacement. Parser messages are presentation
    // evidence only; they never decide whether publication is admissible.
    let same_diagnostic = |a: &parser_diagnostic::Diagnostic, b: &parser_diagnostic::Diagnostic| {
        a.code == b.code && a.severity == b.severity && a.message == b.message && a.notes == b.notes
    };
    let mut retained_parser = 0;
    for (index, diagnostic) in candidate.iter().enumerate() {
        if candidate[..index]
            .iter()
            .filter(|other| same_diagnostic(diagnostic, other))
            .count()
            < baseline
                .iter()
                .filter(|other| same_diagnostic(diagnostic, other))
                .count()
            && diagnostic.severity == parser_diagnostic::Severity::Warning
        {
            retained_parser += 1;
        }
        // Parser recovery can hide a script section; retain its locations once.
        parser_diagnostic::write(std::slice::from_ref(diagnostic), writer)?;
    }
    Ok(retained_parser)
}

pub(super) fn write_candidates(inputs: &[super::Edit], checked: bool) -> io::Result<()> {
    for item in inputs {
        let check = item.candidate.as_ref().expect("candidate attempted");
        if let Ok(candidate) = check {
            if let (Some(baseline), Some(report)) = (&item.baseline, &candidate.report) {
                write_changes(&item.subject, baseline, report, &mut output_cli::stderr())?;
            }
            if !candidate.review_triggers.is_empty() {
                output_cli::stderr().message(
                    HumanLevel::Warn,
                    Some(&item.subject),
                    format_args!(
                        "review required after changing {}: source authenticity, unrefreshed digests, patch applicability and native build are not verified",
                        candidate.review_triggers.join(", ")
                    ),
                )?;
            }
        }
        if checked {
            let admissible = matches!(check, Ok(candidate) if super::static_check_error(item, candidate).is_none());
            output_cli::stderr().message(
                if admissible {
                    HumanLevel::Info
                } else {
                    HumanLevel::Error
                },
                Some(&item.subject),
                format_args!(
                    "{}",
                    if admissible {
                        "admissible"
                    } else {
                        "not admissible"
                    }
                ),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_source_issues_keep_their_distinct_candidate_lines() {
        let source = include_str!("../../tests/fixtures/ed.spec")
            .replace("#!RemoteAsset:  sha256:56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc", "#!RemoteAsset")
            .replace("BuildSystem:", "#!RemoteAsset\nSource1: https://example.org/signature.sig\nBuildSystem:");
        let candidate_source = format!("# Extra candidate line\n{source}");
        let baseline = crate::check::analyze(
            &crate::spec::ParsedSpec::parse(&source),
            crate::check::Policy::Authoring,
            &[],
        );
        let candidate = crate::check::analyze(
            &crate::spec::ParsedSpec::parse(&candidate_source),
            crate::check::Policy::Authoring,
            &[],
        );
        let mut output = Vec::new();
        write_changes(
            Path::new("pkg"),
            &baseline,
            &candidate,
            &mut output_cli::HumanOutput::new(&mut output, false),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("[INFO] pkg: candidate static blockers:"),
            "{output}"
        );
        assert_eq!(output.matches("pkg:").count(), 1, "{output}");
        let sources: Vec<_> = candidate
            .findings()
            .iter()
            .filter(|finding| finding.code == "RPK005")
            .collect();
        assert_eq!(sources.len(), 2);
        for source in sources {
            assert!(
                output.contains(&format!(
                    "[WARN] spec[{}:{}] [RPK005]: inherited 1 issue(s)",
                    source.span.start.0, source.span.start.1
                )),
                "{output}"
            );
        }
        assert_eq!(output.matches("[RPK005]: inherited").count(), 2, "{output}");
    }

    #[test]
    fn parser_errors_never_increment_the_warning_summary() {
        let parsed = crate::spec::ParsedSpec::parse("Name: demo\n%endif\n");
        let report = crate::check::analyze(&parsed, crate::check::Policy::Authoring, &[]);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.severity == parser_diagnostic::Severity::Error)
        );
        assert!(
            !report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.severity == parser_diagnostic::Severity::Warning)
        );
        let mut output = Vec::new();
        write_changes(
            Path::new("WORK"),
            &report,
            &report,
            &mut output_cli::HumanOutput::new(&mut output, false),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("[ERROR]"), "{output:?}");
        assert!(!output.contains("warnings:"), "{output:?}");
    }
}
