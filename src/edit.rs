// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source-preserving edits through TOML fields and external editors.

mod candidate;
mod drafts;
mod editor;
mod error;

pub(crate) use error::EditError;
use error::Kind;
pub(crate) mod fields;
mod options;

pub(crate) use options::Options;

use crate::spec::{ParsedSpec, document::Snapshot};
use crate::{file_output, utf8_file};
use options::CheckFormat;
use serde_json::json;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use toml::Table;

struct Input {
    path: PathBuf,
    snapshot: Snapshot,
    draft: Option<PathBuf>,
}

/// Builds and checks every candidate before publishing any file.
pub(crate) fn run(options: &Options) -> Result<bool, EditError> {
    match execute(options) {
        Err(error) if options.check && matches!(options.format, Some(CheckFormat::Json)) => {
            let text =
                serde_json::to_string_pretty(&json!({"format_version": 2, "scope": "selected-edit-static", "valid": false, "files": [], "error": error}))
                    .map_err(|e| EditError::from(e.to_string()))?;
            write_stdout(&(text + "\n")).map_err(EditError::from)?;
            Ok(false)
        }
        result => result,
    }
}

fn execute(options: &Options) -> Result<bool, EditError> {
    let mut inputs = if let Some(dir) = &options.from {
        drafts::load(dir)?
            .into_iter()
            .map(|draft| input(draft.source, draft.original, draft.fields, Some(draft.path)))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        options
            .specs
            .iter()
            .map(|path| {
                let path =
                    fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let source = utf8_file::read(&path).map_err(|e| e.to_string())?;
                let fields = if options.set.is_empty() {
                    options.field.clone()
                } else {
                    options.set.iter().map(|(field, _)| field.clone()).collect()
                };
                input(path, source, fields, None)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if inputs.is_empty() {
        return Err("no SPEC files selected".into());
    }
    if inputs.len() != 1
        && (options.view || options.schema || options.stdout || options.output.is_some())
    {
        return Err("--view, --schema, --stdout, and --output require exactly one SPEC".into());
    }
    for (i, item) in inputs.iter().enumerate() {
        if inputs[..i].iter().any(|other| other.path == item.path) {
            return Err(format!("{}: SPEC selected more than once", item.path.display()).into());
        }
    }
    if options.view || options.schema {
        let view = inputs[0].snapshot.document();
        let text = if options.schema {
            serde_json::to_string_pretty(&fields::schema(view)).map_err(|e| e.to_string())? + "\n"
        } else {
            toml::to_string_pretty(view).map_err(|e| e.to_string())?
        };
        write_stdout(&text)?;
        return Ok(true);
    }
    if let Some(dir) = &options.prepare {
        let created = create_drafts(dir, &inputs)?;
        let dir = created[0]
            .parent()
            .expect("created drafts have an absolute parent");
        let display = dir.to_string_lossy();
        let quoted = shell_words::quote(&display);
        writeln!(io::stderr().lock(), "Drafts: {display}\nCheck: ruyipack edit --from={quoted} --check\nPreview: ruyipack edit --from={quoted} --diff").map_err(|e| e.to_string())?;
        return Ok(true);
    }
    // Saved drafts already contain the edit; reopening them requires --editor.
    let opens_editor = options.editor.is_some()
        || (options.set.is_empty() && !options.check && options.from.is_none());
    let mut temporary = None;
    if opens_editor {
        if options.from.is_none() {
            let dir = tempfile::Builder::new()
                .prefix("ruyipack-edit-")
                .tempdir()
                .map_err(|e| e.to_string())?;
            let created = create_drafts(dir.path(), &inputs)?;
            for (item, draft) in inputs.iter_mut().zip(created) {
                item.draft = Some(draft);
            }
            temporary = Some(dir);
        }
        let paths = inputs
            .iter()
            .filter_map(|item| item.draft.as_deref())
            .collect::<Vec<_>>();
        if let Err(error) = editor::open(&paths, options.editor.as_deref()) {
            return Err(retain(error.into(), temporary, &inputs));
        }
    }
    let result = apply(options, &inputs).and_then(|result| {
        for outcome in &result.outcomes {
            outcome
                .write_human(&mut io::stderr().lock())
                .map_err(|e| e.to_string())?;
        }
        Ok(result)
    });
    match result {
        Err(error) => Err(retain(error, temporary, &inputs)),
        Ok(result) => {
            // Keep an editor's work when it was only previewed, copied, or declined.
            if let Some(dir) = temporary
                && result.has_unapplied_changes()
            {
                let path = dir.keep();
                writeln!(
                    io::stderr().lock(),
                    "Drafts retained: {}\nResume: ruyipack edit --from={}",
                    path.display(),
                    shell_words::quote(&path.to_string_lossy())
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(result.success)
        }
    }
}

fn input(
    path: PathBuf,
    source: String,
    fields: Vec<String>,
    draft: Option<PathBuf>,
) -> Result<Input, EditError> {
    let parsed = ParsedSpec::parse(&source);
    let snapshot = Snapshot::capture_selected(&parsed, &fields)
    .map_err(|error| {
        let mut message = format!("{}: {error}", path.display());
        if fields.is_empty() && !parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.severity == crate::parser_diagnostic::Severity::Error
        }) {
            let name = path.to_string_lossy();
            let quoted = shell_words::quote(&name);
            message.push_str(&format!(
                "\nFull-view editing requires a mapping for every construct. Select supported fields instead, for example:\n  ruyipack edit {quoted} --field package.version --view\nOmit --view to edit the selected field. Use inspect to read the main-package tags."
            ));
        }
        EditError::at(Kind::UnmappableFields, &path, &fields, message)
    })?;
    fields::validate_selection(snapshot.document(), &fields)
        .map_err(|error| EditError::at(Kind::UnmappableFields, &path, &fields, error))?;
    Ok(Input {
        path,
        snapshot,
        draft,
    })
}

fn create_drafts(dir: &Path, inputs: &[Input]) -> Result<Vec<PathBuf>, String> {
    let documents = inputs
        .iter()
        .map(|item| {
            (
                item.path.as_path(),
                item.snapshot.source(),
                item.snapshot.selection(),
                item.snapshot.document(),
            )
        })
        .collect::<Vec<_>>();
    drafts::create(dir, &documents)
}

// Keep each input attached to its candidate/error; reports and publication consume
// the same result rather than maintaining parallel, partially populated vectors.
struct CheckedInput<'a> {
    input: &'a Input,
    // Static failures retain their candidate report; mapping failures have no candidate.
    candidate: Option<candidate::Candidate>,
    error: Option<EditError>,
}

impl CheckedInput<'_> {
    fn record(&self) -> serde_json::Value {
        let item = self.input;
        let mut record = if let Some(candidate) = self.candidate.as_ref() {
            let review_required: &[&str] = if candidate.review_triggers.is_empty() {
                &[]
            } else {
                &[
                    "source-content-and-digests",
                    "patch-applicability",
                    "native-build",
                ]
            };
            json!({"source": item.path, "draft": item.draft, "valid": self.error.is_none(),
                "original_sha256": utf8_file::digest(item.snapshot.source()), "report_subject": "candidate",
                "profile": crate::profile::identity(), "changed": candidate.contents != item.snapshot.source(),
                "review_triggers": candidate.review_triggers, "review_required": review_required,
                "report": candidate.report.structured(&item.path)})
        } else {
            json!({"source": item.path, "draft": item.draft, "valid": false})
        };
        if let Some(error) = self.error.as_ref() {
            record["error"] = serde_json::to_value(error).expect("serializable error");
        }
        record
    }
}

struct ApplyResult<'a> {
    success: bool,
    changed_sources: Vec<&'a Path>,
    outcomes: Vec<file_output::EditOutcome>,
}

impl ApplyResult<'_> {
    fn has_unapplied_changes(&self) -> bool {
        self.changed_sources.iter().any(|source| !self.outcomes.iter().any(|outcome| {
            matches!(outcome, file_output::EditOutcome::Written(path) | file_output::EditOutcome::Unchanged(path) if path == source)
        }))
    }
}

fn apply<'a>(options: &Options, inputs: &'a [Input]) -> Result<ApplyResult<'a>, EditError> {
    if let Some(output) = &options.output {
        let paths = inputs
            .iter()
            .filter_map(|item| item.draft.as_deref())
            .collect::<Vec<_>>();
        drafts::protect_output(output, &paths)?;
    }
    let checked = inputs
        .iter()
        .map(|item| {
            let (candidate, error) = match read_candidate(item, &options.set) {
                Ok(candidate) => {
                    let error = (!candidate.report.is_success()).then(|| {
                        EditError::at(
                            Kind::StaticCheckFailed,
                            &item.path,
                            item.snapshot.selection(),
                            format!("{}: candidate failed static checks", item.path.display()),
                        )
                    });
                    (Some(candidate), error)
                }
                Err(error) => (None, Some(error)),
            };
            CheckedInput {
                input: item,
                candidate,
                error,
            }
        })
        .collect::<Vec<_>>();
    let success = checked.iter().all(|item| item.error.is_none());
    if !matches!(options.format, Some(CheckFormat::Json)) {
        for item in &checked {
            if let Some(candidate) = item.candidate.as_ref() {
                let path = &item.input.path;
                if !candidate.review_triggers.is_empty() {
                    writeln!(io::stderr().lock(),
                        "{} (candidate): review required after changing {}: source content and recorded SHA-256, patch applicability, and native build have not been verified",
                        path.display(), candidate.review_triggers.join(", "))
                        .map_err(|e| e.to_string())?;
                }
                candidate
                    .report
                    .write_human(
                        Path::new(&format!("{} (candidate)", path.display())),
                        &mut io::stderr().lock(),
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    let changed_sources = checked
        .iter()
        .filter_map(|item| {
            item.candidate
                .as_ref()
                .filter(|candidate| candidate.contents != item.input.snapshot.source())
                .map(|_| item.input.path.as_path())
        })
        .collect();
    if options.check {
        if matches!(options.format, Some(CheckFormat::Json)) {
            let records = checked.iter().map(CheckedInput::record).collect::<Vec<_>>();
            write_stdout(&(serde_json::to_string_pretty(
                &json!({"format_version": 2, "scope": "selected-edit-static", "valid": success, "files": records})
            ).map_err(|e| e.to_string())? + "\n"))?;
        } else {
            for item in &checked {
                writeln!(
                    io::stdout().lock(),
                    "{}: {}",
                    item.input.path.display(),
                    if item.error.is_none() {
                        "valid"
                    } else {
                        "invalid"
                    }
                )
                .map_err(|e| e.to_string())?;
            }
            for error in checked.iter().filter_map(|item| item.error.as_ref()) {
                writeln!(io::stderr().lock(), "error: {error}").map_err(|e| e.to_string())?;
            }
        }
        return Ok(ApplyResult {
            success,
            changed_sources,
            outcomes: Vec::new(),
        });
    }
    if !success {
        let message = checked
            .iter()
            .filter_map(|item| item.error.as_ref())
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(message.into());
    }
    let files = checked
        .iter()
        .map(|item| {
            let candidate = item
                .candidate
                .as_ref()
                .expect("successful check has a candidate");
            file_output::EditFile {
                source_path: &item.input.path,
                original: item.input.snapshot.source(),
                contents: &candidate.contents,
            }
        })
        .collect::<Vec<_>>();
    let mode = if options.diff {
        file_output::EditMode::Diff
    } else if options.stdout {
        file_output::EditMode::Stdout
    } else if options.force {
        file_output::EditMode::Overwrite
    } else {
        file_output::EditMode::Write
    };
    let outcomes = file_output::run_edits(
        &files,
        options.output.as_deref(),
        mode,
        crate::output_cli::select_edit_action,
    )
    .map_err(EditError::publication)?;
    Ok(ApplyResult {
        success,
        changed_sources,
        outcomes,
    })
}

fn read_candidate(
    item: &Input,
    assignments: &[(String, String)],
) -> Result<candidate::Candidate, EditError> {
    if !utf8_file::is_unchanged(&item.path, item.snapshot.source())
        .map_err(|e| format!("{}: {e}", item.path.display()))?
    {
        return Err(EditError::at(
            Kind::SourceChanged,
            &item.path,
            item.snapshot.selection(),
            format!(
                "{}: source changed; prepare a fresh draft",
                item.path.display()
            ),
        ));
    }
    let document = if let Some(path) = &item.draft {
        let text = drafts::read_text(path)?;
        let edited: Table = toml::from_str(&text).map_err(|e: toml::de::Error| {
            let offset = e.span().map_or(0, |span| span.start).min(text.len());
            let before = &text[..offset];
            let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
            let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            EditError::at(
                Kind::InvalidDraft,
                path,
                item.snapshot.selection(),
                format!("{}:{line}:{column}: {}", path.display(), e.message()),
            )
        })?;
        edited
    } else {
        fields::assign(item.snapshot.document(), assignments).map_err(|e| {
            EditError::at(
                Kind::InvalidAssignment,
                &item.path,
                item.snapshot.selection(),
                e,
            )
        })?
    };
    candidate::prepare(&item.snapshot, &document).map_err(|error| {
        let path = item.draft.as_deref().unwrap_or(&item.path);
        EditError::at(
            Kind::InvalidCandidate,
            path,
            item.snapshot.selection(),
            format!("{}: {error}", path.display()),
        )
    })
}

fn retain(
    mut error: EditError,
    temporary: Option<tempfile::TempDir>,
    inputs: &[Input],
) -> EditError {
    error.message = match temporary {
        Some(dir) => {
            let path = dir.keep();
            if error.invalidates_drafts(
                &inputs
                    .iter()
                    .map(|item| item.path.as_path())
                    .collect::<Vec<_>>(),
            ) || inputs.iter().any(|item| {
                !utf8_file::is_unchanged(&item.path, item.snapshot.source()).unwrap_or(false)
            }) {
                format!(
                    "{error}\nDrafts retained: {}\nSources changed; review the written files and prepare fresh drafts before retrying.",
                    path.display()
                )
            } else {
                format!(
                    "{error}\nDrafts retained: {}\nResume: ruyipack edit --from={}",
                    path.display(),
                    shell_words::quote(&path.to_string_lossy())
                )
            }
        }
        None => error.message,
    };
    error
}
fn write_stdout(text: &str) -> Result<(), String> {
    io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|e| format!("failed to write output to stdout: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_output::{EditOutcome, OutputError};

    #[test]
    fn recovery_uses_changed_sources_and_actual_publication_destinations() {
        let source = Path::new("source.spec");
        let copy = Path::new("source.spec.new");
        for (outcomes, expected) in [
            (vec![], true), // Read-only preview.
            (vec![EditOutcome::Skipped(source.into())], true),
            (vec![EditOutcome::Written(copy.into())], true),
            (vec![EditOutcome::Unchanged(copy.into())], true),
            (vec![EditOutcome::Written(source.into())], false),
            (vec![EditOutcome::Unchanged(source.into())], false),
        ] {
            let result = ApplyResult {
                success: true,
                changed_sources: vec![source],
                outcomes,
            };
            assert_eq!(result.has_unapplied_changes(), expected);
        }
        let unchanged = ApplyResult {
            success: true,
            changed_sources: vec![],
            outcomes: vec![],
        };
        assert!(!unchanged.has_unapplied_changes());
        let batch = ApplyResult {
            success: true,
            changed_sources: vec![source, Path::new("second.spec")],
            outcomes: vec![EditOutcome::Written(source.into())],
        };
        assert!(batch.has_unapplied_changes());
    }

    #[test]
    fn partial_publication_facts_survive_even_if_source_bytes_are_restored() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("first, with spaces.spec");
        let original = include_str!("../tests/fixtures/ed.spec");
        fs::write(&source, original).unwrap();
        let inputs = vec![
            input(
                source.clone(),
                original.into(),
                vec!["package.version".into()],
                None,
            )
            .unwrap(),
        ];
        let temporary = tempfile::tempdir_in(work.path()).unwrap();
        let retained = temporary.path().to_owned();
        let error = EditError::publication(OutputError::Partial {
            written: vec![source],
            source: Box::new(OutputError::Write {
                path: work.path().join("second.spec"),
                source: io::Error::from(io::ErrorKind::PermissionDenied),
            }),
        });
        assert!(std::error::Error::source(&error).is_some());
        let error = retain(error, Some(temporary), &inputs);
        assert!(error.message.contains("Sources changed;"));
        assert!(!error.message.contains("Resume:"));
        assert!(retained.is_dir());
        let copy_failure = EditError::publication(OutputError::Partial {
            written: vec![work.path().join("copy.spec")],
            source: Box::new(OutputError::Selection(Box::new(io::Error::from(
                io::ErrorKind::Interrupted,
            )))),
        });
        assert!(!copy_failure.invalidates_drafts(&[inputs[0].path.as_path()]));
        let stale = EditError::publication(OutputError::SourceChanged(inputs[0].path.clone()));
        assert!(stale.invalidates_drafts(&[inputs[0].path.as_path()]));
    }
}
