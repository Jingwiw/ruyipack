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
mod fields;
mod options;

pub(crate) use options::Options;

use crate::output_cli::ReportFormat;
use crate::spec::{ParsedSpec, document::Snapshot};
use crate::{file_output, utf8_file};
use serde_json::json;
use std::{
    borrow::Cow,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use toml::Table;

struct Input {
    path: PathBuf,
    snapshot: Snapshot,
    baseline: crate::check_report::CheckReport,
    draft: Option<PathBuf>,
}

/// Builds and checks every candidate before publishing any file.
pub(crate) fn run(mut options: Options) -> Result<bool, EditError> {
    if options.from.is_none()
        && !options.all
        && options.fields.is_empty()
        && options.set.is_empty()
        && options.hash_sources.is_empty()
        && options.format.is_none()
        && !options.check
        && !options.view
        && !options.schema
        && let Some(field) = crate::output_cli::select_edit_field()?
    {
        options.fields.push(field.to_owned());
    }
    let options = &options;
    let mut report = json!({"format_version": 2, "scope": if options.prepare.is_some() { "edit-draft" } else { "selected-edit-static" }, "files": [],
        "operation": if options.prepare.is_some() { "prepare" } else if options.check { "check" } else { "apply" }});
    let result = execute(options, &mut report);
    if !matches!(options.format, Some(ReportFormat::Json)) {
        return result.map(|(success, _)| success);
    }
    let (success, written) = match result {
        Ok((success, outcomes)) => (
            success,
            outcomes
                .into_iter()
                .filter_map(|outcome| match outcome {
                    file_output::EditOutcome::Written(path) => Some(path),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        ),
        Err(error) => {
            let written = error.written_paths().to_vec();
            report["error"] = serde_json::to_value(error).expect("serializable error");
            (false, written)
        }
    };
    report["valid"] = success.into();
    // A failed report write is an I/O error, not a new business result to serialize.
    let text = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    write_stdout(&(text + "\n")).map_err(|mut error| {
        if !written.is_empty() {
            error.push_str(&format!("\nFiles already written: {written:?}\nInspect these files and their current SHA-256 before retrying; publication was not rolled back."));
        }
        error
    })?;
    Ok(success)
}

fn load_inputs(options: &Options) -> Result<Vec<Input>, EditError> {
    if options.from.is_none()
        && !options.all
        && options.fields.is_empty()
        && options.set.is_empty()
        && options.hash_sources.is_empty()
    {
        return Err("select what to edit: use --field package.version (opens the editor), --set package.version=VERSION, or --all for a fully supported SPEC. For a read-only overview, use inspect".into());
    }
    let inputs = if let Some(dir) = &options.from {
        drafts::load(dir)?
            .into_iter()
            .map(|draft| input(draft.source, draft.original, draft.fields, Some(draft.path)))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        options
            .specs
            .iter()
            .map(|path| {
                let mut fields = if options.set.is_empty() {
                    options.fields.clone()
                } else {
                    options.set.iter().map(|(field, _)| field.clone()).collect()
                };
                if !options.all {
                    for number in &options.hash_sources {
                        let field = format!("sources.{number}.sha256");
                        if !fields.contains(&field) {
                            fields.push(field);
                        }
                    }
                }
                let path = fs::canonicalize(path).map_err(|e| {
                    EditError::at(
                        Kind::InputRead,
                        path,
                        &fields,
                        format!("{}: {e}", path.display()),
                    )
                })?;
                let source = utf8_file::read(&path)
                    .map_err(|e| EditError::at(Kind::InputRead, &path, &fields, e.to_string()))?;
                input(path, source, fields, None)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if let Some(expected) = &options.expect_sha256 {
        let [item] = inputs.as_slice() else {
            return Err("--expect-sha256 requires exactly one SPEC".into());
        };
        let actual = utf8_file::sha256(item.snapshot.source());
        if &actual != expected {
            return Err(EditError::at(
                Kind::SourceChanged,
                &item.path,
                item.snapshot.selection(),
                format!(
                    "{}: expected SHA-256 {expected}, found {actual}; no files written",
                    item.path.display()
                ),
            ));
        }
    }
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
    Ok(inputs)
}

fn execute(
    options: &Options,
    report: &mut serde_json::Value,
) -> Result<(bool, Vec<file_output::EditOutcome>), EditError> {
    let mut inputs = load_inputs(options)?;
    if options.view || options.schema {
        let view = inputs[0].snapshot.document();
        let text = if options.schema {
            serde_json::to_string_pretty(&fields::schema(view)).map_err(|e| e.to_string())? + "\n"
        } else {
            toml::to_string_pretty(view).map_err(|e| e.to_string())?
        };
        write_stdout(&text)?;
        return Ok((true, Vec::new()));
    }
    if let Some(dir) = &options.prepare {
        let created = create_drafts(dir, &inputs, Some(options))?;
        if matches!(options.format, Some(ReportFormat::Json)) {
            report["files"] = inputs.iter().zip(&created).map(|(item, path)| json!({
                "source": item.path.to_string_lossy(), "original_sha256": utf8_file::sha256(item.snapshot.source()),
                "draft": path.to_string_lossy(),
            })).collect();
            return Ok((true, Vec::new()));
        }
        let dir = created[0]
            .parent()
            .expect("created drafts have an absolute parent");
        let display = dir.to_string_lossy();
        let quoted = shell_words::quote(&display);
        writeln!(io::stderr().lock(), "Drafts: {display}\nCheck: ruyipack edit --from={quoted} --check\nPreview: ruyipack edit --from={quoted} --diff").map_err(|e| e.to_string())?;
        return Ok((true, Vec::new()));
    }
    // Saved drafts already contain the edit; reopening them requires --editor.
    let opens_editor = options.editor.is_some()
        || (options.set.is_empty()
            && options.hash_sources.is_empty()
            && !options.check
            && options.from.is_none());
    if opens_editor && matches!(options.format, Some(ReportFormat::Json)) {
        return Err("JSON reports require an explicit non-interactive action: --set, --hash-source, --prepare, --from, or --check".into());
    }
    let mut temporary = None;
    if opens_editor {
        for item in &inputs {
            if !item.baseline.is_success() {
                writeln!(io::stderr().lock(), "{}: pre-existing static issues must be resolved before publication; the editor may repair them", item.path.display()).map_err(|e| e.to_string())?;
                item.baseline
                    .write_human(&item.path, &mut io::stderr().lock())
                    .map_err(|e| e.to_string())?;
            }
        }
        if options.from.is_none() {
            let dir = tempfile::Builder::new()
                .prefix("ruyipack-edit-")
                .tempdir()
                .map_err(|e| e.to_string())?;
            let created = create_drafts(dir.path(), &inputs, None)?;
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
    match apply(options, &inputs, report) {
        Err(error) => Err(retain(error, temporary, &inputs)),
        Ok(result) => {
            // Finalize drafts from publication facts before fallible notifications.
            let retained = temporary
                .filter(|_| result.has_unapplied_changes())
                .map(tempfile::TempDir::keep);
            for outcome in result
                .outcomes
                .iter()
                .filter(|_| !matches!(options.format, Some(ReportFormat::Json)))
            {
                outcome
                    .write_human(&mut io::stderr().lock())
                    .map_err(|e| e.to_string())?;
            }
            if let Some(path) = retained {
                writeln!(
                    io::stderr().lock(),
                    "Drafts retained: {}\nResume: ruyipack edit --from={}",
                    path.display(),
                    shell_words::quote(&path.to_string_lossy())
                )
                .map_err(|e| e.to_string())?;
            }
            Ok((result.success, result.outcomes))
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
        baseline: crate::check::analyze(&parsed),
        draft,
    })
}

fn create_drafts(
    dir: &Path,
    inputs: &[Input],
    options: Option<&Options>,
) -> Result<Vec<PathBuf>, String> {
    let documents = inputs
        .iter()
        .map(|item| {
            let mut document = Cow::Borrowed(item.snapshot.document());
            if let Some(options) = options.filter(|options| !options.hash_sources.is_empty()) {
                complete_hashes(item, document.to_mut(), options).map_err(|e| e.to_string())?;
            }
            Ok(document)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sources = inputs
        .iter()
        .zip(&documents)
        .map(|(item, document)| {
            (
                item.path.as_path(),
                item.snapshot.source(),
                item.snapshot.selection(),
                document.as_ref(),
            )
        })
        .collect::<Vec<_>>();
    drafts::create(dir, &sources)
}

fn candidate_record(item: &Input, candidate: &candidate::Candidate) -> serde_json::Value {
    let review_required: &[&str] = if candidate.review_triggers.is_empty() {
        &[]
    } else {
        &[
            "source-content-and-digests",
            "patch-applicability",
            "native-build",
        ]
    };
    json!({"source": item.path.to_string_lossy(), "draft": item.draft.as_deref().map(Path::to_string_lossy),
        "valid": candidate.report.is_success(),
        "original_sha256": utf8_file::sha256(item.snapshot.source()), "report_subject": "candidate",
        "profile": crate::profile::identity(), "changed": candidate.contents != item.snapshot.source(),
        "review_triggers": candidate.review_triggers, "review_required": review_required,
        "source_hashes": candidate.source_hashes,
        "baseline_report": item.baseline.structured(&item.path),
        "introduced_static_blockers": candidate.report.introduced_static_blockers(&item.baseline),
        "report": candidate.report.structured(&item.path)})
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

fn apply<'a>(
    options: &Options,
    inputs: &'a [Input],
    report: &mut serde_json::Value,
) -> Result<ApplyResult<'a>, EditError> {
    if let Some(output) = &options.output {
        let paths = inputs
            .iter()
            .filter_map(|item| item.draft.as_deref())
            .collect::<Vec<_>>();
        drafts::protect_output(output, &paths)?;
    }
    let checked = inputs
        .iter()
        .map(|item| read_candidate(item, options))
        .collect::<Vec<_>>();
    let json = matches!(options.format, Some(ReportFormat::Json));
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let mut changed_sources = Vec::new();
    for (item, result) in inputs.iter().zip(&checked) {
        let candidate = result.as_ref().ok();
        let static_error = candidate.filter(|c| !c.report.is_success()).map(|candidate| {
            EditError::at(
                Kind::StaticCheckFailed,
                &item.path,
                item.snapshot.selection(),
                format!("{}: candidate failed static checks: {}. No SPEC files written; repair the reported fields and retry", item.path.display(),
                    match candidate.report.introduced_static_blockers(&item.baseline) {
                        Some(true) => "new or changed static blockers (compare baseline_report in --check --format json)",
                        Some(false) => "pre-existing blockers remain; no new static failures introduced",
                        None => "static checks remain incomplete; unresolved results cannot be attributed to this edit",
                    }),
            )
        });
        let error = result.as_ref().err().or(static_error.as_ref());
        if json {
            let mut record = candidate.map_or_else(
                || json!({"source": item.path.to_string_lossy(), "draft": item.draft.as_deref().map(Path::to_string_lossy), "valid": false}),
                |candidate| candidate_record(item, candidate),
            );
            if let Some(error) = error {
                record["error"] = serde_json::to_value(error).expect("serializable error");
            }
            records.push(record);
        }
        if let Some(candidate) = candidate {
            if candidate.contents != item.snapshot.source() {
                changed_sources.push(item.path.as_path());
            }
            if !json {
                if !candidate.review_triggers.is_empty() {
                    writeln!(io::stderr().lock(),
                    "{} (candidate): review required after changing {}: source authenticity, unrefreshed digests, patch applicability, and native build have not been verified",
                    item.path.display(), candidate.review_triggers.join(", "))
                    .map_err(|e| e.to_string())?;
                }
                candidate
                    .report
                    .write_human(
                        Path::new(&format!("{} (candidate)", item.path.display())),
                        &mut io::stderr().lock(),
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        if options.check && !json {
            writeln!(
                io::stdout().lock(),
                "{}: {}",
                item.path.display(),
                if error.is_none() { "valid" } else { "invalid" }
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(error) = error {
            errors.push(error.to_string());
        }
    }
    if json {
        report["files"] = records.into();
    }
    let success = errors.is_empty();
    if options.check {
        if !json {
            for error in &errors {
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
        return Err(errors.join("\n").into());
    }
    let candidates = checked.into_iter().collect::<Result<Vec<_>, _>>()?;
    let files = inputs
        .iter()
        .zip(&candidates)
        .map(|(item, candidate)| file_output::EditFile {
            source_path: &item.path,
            original: item.snapshot.source(),
            contents: &candidate.contents,
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
    let publication = file_output::run_edits(
        &files,
        options.output.as_deref(),
        mode,
        crate::output_cli::select_edit_action,
    );
    if json {
        match &publication {
            Ok(outcomes) => {
                report["outcomes"] = outcomes.iter().zip(&files).map(|(outcome, file)| {
                    let (status, path, sha256) = match outcome {
                        file_output::EditOutcome::Written(path) => ("written", path, Some(utf8_file::sha256(file.contents))),
                        file_output::EditOutcome::Unchanged(path) => ("unchanged", path, Some(utf8_file::sha256(file.contents))),
                        file_output::EditOutcome::Skipped(path) => ("skipped", path, None),
                    };
                    json!({"source": file.source_path.to_string_lossy(), "status": status, "path": path.to_string_lossy(), "sha256": sha256})
                }).collect();
            }
            Err(file_output::OutputError::Partial { written, .. }) => {
                report["written"] = json!(
                    written
                        .iter()
                        .map(|path| path.to_string_lossy())
                        .collect::<Vec<_>>()
                )
            }
            Err(_) => {}
        }
    }
    let outcomes = publication.map_err(EditError::publication)?;
    Ok(ApplyResult {
        success,
        changed_sources,
        outcomes,
    })
}

fn read_candidate(item: &Input, options: &Options) -> Result<candidate::Candidate, EditError> {
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
    let mut document = if let Some(path) = &item.draft {
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
        Cow::Owned(edited)
    } else {
        fields::assign(item.snapshot.document(), &options.set).map_err(|e| {
            EditError::at(
                Kind::InvalidAssignment,
                &item.path,
                item.snapshot.selection(),
                e,
            )
        })?
    };
    let source_hashes = if options.hash_sources.is_empty() {
        None
    } else {
        Some(complete_hashes(item, document.to_mut(), options)?)
    };
    let mut candidate = candidate::prepare(&item.snapshot, &document).map_err(|error| {
        let path = item.draft.as_deref().unwrap_or(&item.path);
        EditError::at(
            Kind::InvalidCandidate,
            path,
            item.snapshot.selection(),
            format!("{}: {error}", path.display()),
        )
    })?;
    candidate.source_hashes = source_hashes;
    Ok(candidate)
}

fn complete_hashes(
    item: &Input,
    document: &mut Table,
    options: &Options,
) -> Result<crate::source::SourceHashes, EditError> {
    let complete = || -> Result<crate::source::SourceHashes, String> {
        // A saved draft never silently acquires permission to edit another field.
        for number in &options.hash_sources {
            let field = format!("sources.{number}.sha256");
            if crate::spec::document::table::lookup(document, &field).is_none() {
                return Err(format!(
                    "{field}: prepare a draft that includes this digest field"
                ));
            }
            if options.set.iter().any(|(key, _)| key == &field) {
                return Err(format!(
                    "{field}: choose either --set or --hash-source, not both"
                ));
            }
        }
        let contents = item.snapshot.render_before_hashing(document)?;
        let hashes = crate::source::calculate(
            &item.path,
            &contents,
            &options.hash_sources,
            &options.defines,
        )?;
        crate::source::ensure_unchanged(&item.path, item.snapshot.source())?;
        Ok(hashes)
    };
    let hashes = complete().map_err(|e| {
        EditError::at(
            Kind::SourceHashFailed,
            &item.path,
            item.snapshot.selection(),
            e,
        )
    })?;
    for (number, source) in &hashes.sources {
        *crate::spec::document::table::lookup_mut(document, &format!("sources.{number}.sha256"))
            .expect("selected digest was checked before download") =
            toml::Value::String(source.sha256.clone());
    }
    Ok(hashes)
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
