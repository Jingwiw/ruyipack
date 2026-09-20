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
    source: String,
    snapshot: Snapshot,
    fields: Vec<String>,
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
        let view = fields::select(inputs[0].snapshot.document(), &inputs[0].fields)?;
        let text = if options.schema {
            serde_json::to_string_pretty(&fields::schema(&view)).map_err(|e| e.to_string())? + "\n"
        } else {
            toml::to_string_pretty(&view).map_err(|e| e.to_string())?
        };
        write_stdout(&text)?;
        return Ok(true);
    }
    if let Some(dir) = &options.prepare {
        let created = create_drafts(dir, &inputs)?;
        let dir = created[0]
            .path
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
                item.draft = Some(draft.path);
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
    let result = apply(options, &inputs);
    match result {
        Err(error) => Err(retain(error, temporary, &inputs)),
        Ok(success) => {
            // Keep an editor's work when it was only previewed, copied, or declined.
            if let Some(dir) = temporary {
                let unapplied = inputs.iter().any(|item| {
                    let Some(path) = &item.draft else {
                        return false;
                    };
                    let Ok(text) = fs::read_to_string(path) else {
                        return true;
                    };
                    let Ok(table) = toml::from_str::<Table>(&text) else {
                        return true;
                    };
                    let Ok(expected) = fields::select(item.snapshot.document(), &item.fields)
                    else {
                        return true;
                    };
                    table != expected
                        && fs::read_to_string(&item.path).is_ok_and(|now| now == item.source)
                });
                if unapplied {
                    let path = dir.keep();
                    writeln!(
                        io::stderr().lock(),
                        "Drafts retained: {}\nResume: ruyipack edit --from={}",
                        path.display(),
                        shell_words::quote(&path.to_string_lossy())
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok(success)
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
    let snapshot = if fields.is_empty() {
        Snapshot::capture(&parsed)
    } else {
        Snapshot::capture_selected(&parsed, &fields)
    }
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
    fields::select(snapshot.document(), &fields)
        .map_err(|error| EditError::at(Kind::UnmappableFields, &path, &fields, error))?;
    Ok(Input {
        path,
        source,
        snapshot,
        fields,
        draft,
    })
}

fn create_drafts(dir: &Path, inputs: &[Input]) -> Result<Vec<drafts::Draft>, String> {
    let documents = inputs
        .iter()
        .map(|item| {
            Ok((
                item.path.clone(),
                item.source.clone(),
                item.fields.clone(),
                fields::select(item.snapshot.document(), &item.fields)?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    drafts::create(dir, &documents)
}

fn apply(options: &Options, inputs: &[Input]) -> Result<bool, EditError> {
    if let Some(output) = &options.output {
        protect_drafts(output, inputs)?;
    }
    let mut candidates = Vec::with_capacity(inputs.len());
    let json_output = options.check && matches!(options.format, Some(CheckFormat::Json));
    let mut records = Vec::new();
    let mut valid = Vec::with_capacity(inputs.len());
    let mut errors = Vec::new();
    for item in inputs {
        match read_candidate(item, &options.set) {
            Ok(candidate::Candidate {
                contents: text,
                report,
                review_triggers,
            }) => {
                let success = report.is_success();
                let label = format!("{} (candidate)", item.path.display());
                valid.push(success);
                let review_required: &[&str] = if review_triggers.is_empty() {
                    &[]
                } else {
                    &[
                        "source-content-and-digests",
                        "patch-applicability",
                        "native-build",
                    ]
                };
                if json_output {
                    records.push(
                        json!({"source": item.path, "draft": item.draft, "valid": success,
                        "original_sha256": utf8_file::digest(&item.source), "report_subject": "candidate",
                        "profile": crate::profile::identity(),
                        "changed": text != item.source, "review_triggers": review_triggers,
                        "review_required": review_required,
                        "report": report.structured(&item.path)}),
                    );
                }
                if !matches!(options.format, Some(CheckFormat::Json)) {
                    if !review_triggers.is_empty() {
                        writeln!(io::stderr().lock(),
                            "{} (candidate): review required after changing {}: source content and recorded SHA-256, patch applicability, and native build have not been verified",
                            item.path.display(), review_triggers.join(", "))
                            .map_err(|e| e.to_string())?;
                    }
                    report
                        .write_human(Path::new(&label), &mut io::stderr().lock())
                        .map_err(|e| e.to_string())?;
                }
                if !success {
                    let error = EditError::at(
                        Kind::StaticCheckFailed,
                        &item.path,
                        &item.fields,
                        format!("{}: candidate failed static checks", item.path.display()),
                    );
                    if json_output {
                        records.last_mut().expect("candidate record")["error"] =
                            serde_json::to_value(&error).expect("serializable error");
                    }
                    errors.push(error);
                }
                candidates.push(text);
            }
            Err(error) => {
                valid.push(false);
                if json_output {
                    records.push(
                        json!({"source":item.path, "draft":item.draft, "valid":false, "error":error}),
                    );
                }
                errors.push(error);
            }
        }
    }
    if options.check {
        if matches!(options.format, Some(CheckFormat::Json)) {
            write_stdout(
                &(serde_json::to_string_pretty(
                    &json!({"format_version": 2, "scope": "selected-edit-static", "valid":errors.is_empty(), "files":records}),
                )
                .map_err(|e| e.to_string())?
                    + "\n"),
            )?;
        } else {
            for (item, valid) in inputs.iter().zip(&valid) {
                writeln!(
                    io::stdout().lock(),
                    "{}: {}",
                    item.path.display(),
                    if *valid { "valid" } else { "invalid" }
                )
                .map_err(|e| e.to_string())?;
            }
            for error in &errors {
                writeln!(io::stderr().lock(), "error: {error}").map_err(|e| e.to_string())?;
            }
        }
        return Ok(errors.is_empty());
    }
    if !errors.is_empty() {
        let message = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut error = errors.remove(0);
        error.message = message;
        return Err(error);
    }
    let files = inputs
        .iter()
        .zip(&candidates)
        .map(|(item, contents)| file_output::EditFile {
            source_path: &item.path,
            original: &item.source,
            target_path: options.output.as_deref().unwrap_or(&item.path),
            contents,
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
    for outcome in file_output::run_edits(&files, mode).map_err(EditError::publication)? {
        outcome
            .write_human(&mut io::stderr().lock())
            .map_err(|e| e.to_string())?;
    }
    Ok(true)
}

fn read_candidate(
    item: &Input,
    assignments: &[(String, String)],
) -> Result<candidate::Candidate, EditError> {
    if !utf8_file::is_unchanged(&item.path, &item.source)
        .map_err(|e| format!("{}: {e}", item.path.display()))?
    {
        return Err(EditError::at(
            Kind::SourceChanged,
            &item.path,
            &item.fields,
            format!(
                "{}: source changed; prepare a fresh draft",
                item.path.display()
            ),
        ));
    }
    let document = if let Some(path) = &item.draft {
        let text = utf8_file::read(path).map_err(|e| e.to_string())?;
        let edited: Table = toml::from_str(&text).map_err(|e: toml::de::Error| {
            let offset = e.span().map_or(0, |span| span.start).min(text.len());
            let before = &text[..offset];
            let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
            let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            EditError::at(
                Kind::InvalidDraft,
                path,
                &item.fields,
                format!("{}:{line}:{column}: {}", path.display(), e.message()),
            )
        })?;
        fields::merge(item.snapshot.document(), &item.fields, &edited).map_err(|e| {
            EditError::at(
                Kind::DraftShape,
                path,
                &item.fields,
                format!("{}: {e}", path.display()),
            )
        })?
    } else {
        fields::assign(item.snapshot.document(), assignments)
            .map_err(|e| EditError::at(Kind::InvalidAssignment, &item.path, &item.fields, e))?
    };
    candidate::prepare(&item.snapshot, &item.fields, &document).map_err(|error| {
        let path = item.draft.as_deref().unwrap_or(&item.path);
        EditError::at(
            Kind::InvalidCandidate,
            path,
            &item.fields,
            format!("{}: {error}", path.display()),
        )
    })
}

/// A destination must not replace the draft or its recovery inputs.
fn protect_drafts(output: &Path, inputs: &[Input]) -> Result<(), String> {
    let Some(draft) = inputs.iter().find_map(|item| item.draft.as_ref()) else {
        return Ok(());
    };
    let root = draft.parent().ok_or("draft has no parent directory")?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let target = fs::canonicalize(parent)
        .map_err(|e| format!("{}: {e}", parent.display()))?
        .join(output.file_name().ok_or("output must name a file")?);
    if target.starts_with(root) {
        return Err(format!(
            "{}: output must be outside the draft directory",
            output.display()
        ));
    }
    #[cfg(unix)]
    if let Ok(target_metadata) = fs::metadata(&target) {
        use std::os::unix::fs::MetadataExt;
        let mut protected = inputs
            .iter()
            .filter_map(|item| item.draft.clone())
            .collect::<Vec<_>>();
        protected.push(root.join(".state/index.json"));
        for index in 0..inputs.len() {
            protected.push(root.join(format!(".state/originals/{index}.spec")));
            protected.push(root.join(format!(".state/schema/{index}.json")));
        }
        for path in protected {
            let metadata = fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if metadata.dev() == target_metadata.dev() && metadata.ino() == target_metadata.ino() {
                return Err(format!(
                    "{}: output aliases a draft or its state",
                    output.display()
                ));
            }
        }
    }
    Ok(())
}

fn retain(
    mut error: EditError,
    temporary: Option<tempfile::TempDir>,
    inputs: &[Input],
) -> EditError {
    error.message = match temporary {
        Some(dir) => {
            let path = dir.keep();
            if inputs
                .iter()
                .any(|item| fs::read_to_string(&item.path).is_ok_and(|now| now != item.source))
            {
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
