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

const UPGRADE_REVIEW: &[&str] = &["source-authenticity", "patch-applicability", "native-build"];

#[derive(Serialize)]
pub(super) struct Envelope<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    scope: &'static str,
    upgrade: Option<&'a crate::check::upgrade::Report>,
    operation: &'static str,
    success: bool,
    valid: Option<bool>,
    files: Vec<File<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    outcomes: Vec<Outcome<'a>>,
    error: Option<&'a super::EditError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    written: Vec<Cow<'a, str>>,
    deleted_materials: Vec<Cow<'a, str>>,
}

#[derive(Serialize)]
struct File<'a> {
    source: Cow<'a, str>,
    created_work: bool,
    draft: Option<Cow<'a, str>>,
    original_sha256: String,
    selected_fields: &'a [String],
    source_hashes: Option<&'a crate::source::SourceHashes>,
    #[serde(flatten)]
    state: State<'a>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum State<'a> {
    PendingEdit {
        changed: Option<bool>,
        error: Option<&'a super::EditError>,
    },
    Candidate(Box<CandidateState<'a>>),
}

#[derive(Serialize)]
struct CandidateState<'a> {
    changed: bool,
    profile: crate::profile::Identity,
    changed_fields: &'a [String],
    review_triggers: &'a [String],
    review_required: &'static [&'static str],
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
                changed_fields: &candidate.changed_fields,
                review_triggers: &candidate.review_triggers,
                review_required: if candidate.review_triggers.is_empty() {
                    &[]
                } else {
                    UPGRADE_REVIEW
                },
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
                changed: item.input_changed,
                error: check.and_then(|check| check.as_ref().err()),
            },
        };
        Self {
            source: item.path.to_string_lossy(),
            created_work: item.created_work,
            draft: item.draft.as_ref().map(|path| path.to_string_lossy()),
            original_sha256: crate::utf8_file::sha256(item.snapshot.source()),
            selected_fields: item.snapshot.selection(),
            source_hashes: item.source_hashes.as_ref(),
            state,
        }
    }
}

pub(super) fn envelope<'a>(
    result: &'a Result<super::EditResult, super::EditError>,
    options: &'a super::Options,
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
        format_version: 5,
        deleted_materials: result
            .as_ref()
            .ok()
            .filter(|r| r.publication.is_ok() && options.apply)
            .map_or_else(Vec::new, |r| {
                r.inputs
                    .iter()
                    .flat_map(|item| {
                        item.result()
                            .ok()
                            .into_iter()
                            .flat_map(|c| &c.removed_materials)
                    })
                    .filter(|(path, _)| !path.exists())
                    .map(|(path, _)| path.to_string_lossy())
                    .collect()
            }),
        upgrade: options.upgrade.as_deref(),
        tool: crate::tool::identity(),
        scope: if options.repair_missing {
            "check"
        } else {
            "edit"
        },
        operation: if options.repair_missing {
            "auto-fix"
        } else if options.apply {
            "apply"
        } else if options.check {
            "check"
        } else if options.diff {
            "diff"
        } else {
            "edit"
        },
        success: result
            .as_ref()
            .is_ok_and(|result| result.success_for(options)),
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
        HumanLevel::Debug,
        Some(path),
        format_args!(
            "static rule violations: new {}, inherited {}, resolved {}",
            count(candidate) - inherited,
            inherited,
            count(baseline) - inherited
        ),
    )?;
    let admissible = candidate.allows_edit(baseline);
    let mut legacy = BTreeMap::new();
    for (finding, retained) in &changes {
        if finding.build_requirements.is_some() {
            finding.write_human(writer)?;
        } else if *retained {
            legacy
                .entry((finding.code, finding.span.start))
                .or_insert((
                    Vec::new(),
                    if admissible {
                        HumanLevel::Warn
                    } else {
                        finding.severity.human_level()
                    },
                ))
                .0
                .push(finding.message.as_str());
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
            HumanLevel::Debug,
            None,
            format_args!(
                "warnings: new {}, inherited {}, resolved {}",
                warning_count(candidate) - inherited_warnings,
                inherited_warnings,
                warning_count(baseline) - inherited_warnings
            ),
        )?;
    }
    for ((code, start), (messages, level)) in legacy {
        writer.diagnostic(
            level,
            Some(start),
            Some(code),
            format_args!(
                "inherited {}: {}; edit={}",
                messages.len(),
                messages.join("; "),
                if admissible { "allowed" } else { "blocked" }
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
                "Source digests: refresh with edit --hash-source N; compare with source verify."
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
            if inputs.len() > 1 || candidate.spec.source() != item.snapshot.source() {
                output_cli::stderr().message(
                    HumanLevel::Info,
                    Some(&item.subject),
                    format_args!("candidate"),
                )?;
            }
            if let (Some(baseline), Some(report)) = (&item.baseline, &candidate.report) {
                write_changes(&item.subject, baseline, report, &mut output_cli::stderr())?;
            }
            if !candidate.changed_fields.is_empty() {
                output_cli::stderr().message(
                    HumanLevel::Info,
                    Some(&item.subject),
                    format_args!("candidate changed {}", candidate.changed_fields.join(", ")),
                )?;
            }
            if let Some(hashes) = &item.source_hashes {
                output_cli::stderr().message(
                    HumanLevel::Debug,
                    Some(&item.subject),
                    format_args!(
                        "digest computation: sources={:?}",
                        hashes.sources.keys().collect::<Vec<_>>()
                    ),
                )?;
            }
            if !candidate.review_triggers.is_empty() {
                output_cli::stderr().message(
                    HumanLevel::Warn,
                    Some(&item.subject),
                    format_args!(
                        "unverified {}; triggers={}",
                        UPGRADE_REVIEW.join(", "),
                        candidate.review_triggers.join(", ")
                    ),
                )?;
                if item.source_hashes.is_none() {
                    output_cli::stderr().message(
                        HumanLevel::Warn,
                        Some(&item.subject),
                        format_args!("source-digests: not-refreshed"),
                    )?;
                }
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
                        "check: passed"
                    } else {
                        "check: failed"
                    }
                ),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn failed_application_keeps_observed_upgrade() {
        let options = super::super::Options {
            upgrade: Some(Box::new(crate::check::upgrade::Report {
                project: Some("example".into()),
                current: Some("1".into()),
                candidate: Some("2".into()),
                status: "upgrade-available",
                error: None,
                input_sha256: "baseline".into(),
            })),
            ..Default::default()
        };
        let result = Err(super::super::EditError::from("download failed"));
        let text = toml::to_string(&super::envelope(&result, &options)).unwrap();
        let report: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(report["upgrade"]["candidate"].as_str(), Some("2"));
        assert_eq!(report["success"].as_bool(), Some(false));
        assert!(report.contains_key("error"));
    }
    use super::*;

    #[test]
    fn local_admission_and_resolved_warnings_determine_human_levels() {
        let source = include_str!("../../tests/fixtures/ed.spec");
        let legacy = source.replace("https://www.gnu.org/software/ed/", "ftp://example.org/");
        let baseline_spec = crate::spec::ParsedSpec::parse(&legacy);
        let clean_spec = crate::spec::ParsedSpec::parse(source);
        let baseline = crate::check::analyze(&baseline_spec, crate::check::Policy::Authoring, &[]);
        let clean = crate::check::analyze(&clean_spec, crate::check::Policy::Authoring, &[]);
        for (before, after, allowed) in [(&baseline, &baseline, true), (&clean, &baseline, false)] {
            assert_eq!(after.allows_edit(before), allowed);
            let mut bytes = Vec::new();
            let mut writer = output_cli::HumanOutput::new(&mut bytes, false);
            write_changes(Path::new("pkg"), before, after, &mut writer).unwrap();
            let output = String::from_utf8(bytes).unwrap();
            assert_eq!(output.contains("[ERROR]"), !allowed, "{output}");
            assert_eq!(output.contains("edit=allowed"), allowed, "{output}");
        }
        let missing = source.replace(
            "#!RemoteAsset:  sha256:56e107ddc2f29dad6690376c15bf9751509e1ee3b8241710e44edbe5c3a158cc",
            "#!RemoteAsset",
        );
        let parsed = crate::spec::ParsedSpec::parse(&missing);
        let before = crate::check::analyze(&parsed, crate::check::Policy::Authoring, &[]);
        let mut bytes = Vec::new();
        write_changes(
            Path::new("pkg"),
            &before,
            &clean,
            &mut output_cli::HumanOutput::new(&mut bytes, false),
        )
        .unwrap();
        let output = String::from_utf8(bytes).unwrap();
        assert!(!output.contains("[WARN]"), "{output}");
    }

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
        let sources: Vec<_> = candidate
            .findings()
            .iter()
            .filter(|finding| finding.code == "RPK005")
            .collect();
        assert_eq!(sources.len(), 2);
        for source in sources {
            assert!(
                output.contains(&format!(
                    "[WARN] spec[{}:{}] [RPK005]: inherited 1:",
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
