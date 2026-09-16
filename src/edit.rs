// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source-preserving edits through TOML fields and external editors.

mod document;
mod drafts;
mod fields;
mod options;

pub(crate) use options::Options;

use crate::{check, file_output, utf8_file};
use document::Snapshot;
use options::CheckFormat;
use rpm_spec::parser::parse_str_with_spans;
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
                serde_json::to_string_pretty(&json!({"valid": false, "files": [], "error": error}))
                    .map_err(|e| EditError(e.to_string()))?;
            write_stdout(&(text + "\n")).map_err(EditError)?;
            Ok(false)
        }
        result => result.map_err(EditError),
    }
}

fn execute(options: &Options) -> Result<bool, String> {
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
                let path =
                    fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let source = utf8_file::read(&path).map_err(|e| e.to_string())?;
                input(path, source, options.field.clone(), None)
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
            return Err(format!(
                "{}: SPEC selected more than once",
                item.path.display()
            ));
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
        create_drafts(dir, &inputs)?;
        writeln!(io::stderr().lock(), "Drafts: {}\nCheck: ruyipack edit --from '{}' --check\nPreview: ruyipack edit --from '{}' --diff", dir.display(), dir.display(), dir.display()).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if options.from.is_none() && options.set.is_empty() && !options.check {
        return Err("choose --view, --schema, --set FIELD=VALUE, or --prepare DIR".into());
    }
    apply(options, &inputs)
}

fn input(
    path: PathBuf,
    source: String,
    fields: Vec<String>,
    draft: Option<PathBuf>,
) -> Result<Input, String> {
    let parsed = parse_str_with_spans(&source);
    let snapshot =
        Snapshot::capture(&source, &parsed).map_err(|e| format!("{}: {e}", path.display()))?;
    fields::select(snapshot.document(), &fields)?;
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

fn apply(options: &Options, inputs: &[Input]) -> Result<bool, String> {
    if let Some(output) = &options.output {
        protect_drafts(output, inputs)?;
    }
    let mut candidates = Vec::with_capacity(inputs.len());
    let mut records = Vec::with_capacity(inputs.len());
    let mut errors = Vec::new();
    for item in inputs {
        match candidate(item, &options.set) {
            Ok(text) => {
                let report = check::analyze(&text, parse_str_with_spans(&text));
                let success = report.is_success();
                let label = format!("{} (candidate)", item.path.display());
                let mut serialized = Vec::new();
                report
                    .write_json(Path::new(&label), &mut serialized)
                    .map_err(|e| e.to_string())?;
                records.push(json!({"source": item.path, "draft": item.draft, "valid": success,
                    "changed": text != item.source, "report": serde_json::from_slice::<serde_json::Value>(&serialized).map_err(|e| e.to_string())?}));
                if !matches!(options.format, Some(CheckFormat::Json)) {
                    report
                        .write_human(Path::new(&label), &mut io::stderr().lock())
                        .map_err(|e| e.to_string())?;
                }
                if !success {
                    errors.push(format!(
                        "{}: candidate failed static checks",
                        item.path.display()
                    ));
                }
                candidates.push(text);
            }
            Err(error) => {
                records.push(
                    json!({"source":item.path, "draft":item.draft, "valid":false, "error":error}),
                );
                errors.push(error);
            }
        }
    }
    if options.check {
        if matches!(options.format, Some(CheckFormat::Json)) {
            write_stdout(
                &(serde_json::to_string_pretty(
                    &json!({"valid":errors.is_empty(), "files":records}),
                )
                .map_err(|e| e.to_string())?
                    + "\n"),
            )?;
        } else {
            for record in &records {
                writeln!(
                    io::stdout().lock(),
                    "{}: {}",
                    record["source"].as_str().unwrap_or(""),
                    if record["valid"] == true {
                        "valid"
                    } else {
                        "invalid"
                    }
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
        return Err(errors.join("\n"));
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
        file_output::EditMode::Prompt
    };
    file_output::run_edits(&files, mode).map_err(|e| e.to_string())?;
    Ok(true)
}

fn candidate(item: &Input, assignments: &[(String, String)]) -> Result<String, String> {
    if fs::canonicalize(&item.path).map_err(|e| format!("{}: {e}", item.path.display()))?
        != item.path
        || fs::read_to_string(&item.path).map_err(|e| format!("{}: {e}", item.path.display()))?
            != item.source
    {
        return Err(format!(
            "{}: source changed; prepare a fresh draft",
            item.path.display()
        ));
    }
    let document = if let Some(path) = &item.draft {
        let text = utf8_file::read(path).map_err(|e| e.to_string())?;
        let edited: Table = toml::from_str(&text).map_err(|e: toml::de::Error| {
            let offset = e.span().map_or(0, |span| span.start).min(text.len());
            let before = &text[..offset];
            let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
            let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            format!("{}:{line}:{column}: {}", path.display(), e.message())
        })?;
        fields::merge(item.snapshot.document(), &item.fields, &edited)
            .map_err(|e| format!("{}: {e}", path.display()))?
    } else {
        fields::assign(item.snapshot.document(), assignments)?
    };
    let rendered = item.snapshot.render(&document).map_err(|e| {
        format!(
            "{}: {e}",
            item.draft.as_deref().unwrap_or(&item.path).display()
        )
    })?;
    let parsed = parse_str_with_spans(&rendered);
    let observed = Snapshot::capture(&rendered, &parsed).map_err(|e| {
        format!(
            "{}: {e}",
            item.draft.as_deref().unwrap_or(&item.path).display()
        )
    })?;
    if observed.document() != &document {
        return Err(format!(
            "{}: edited fields did not survive SPEC parsing",
            item.path.display()
        ));
    }
    Ok(rendered)
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

fn write_stdout(text: &str) -> Result<(), String> {
    io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|e| format!("failed to write output to stdout: {e}"))
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct EditError(String);
