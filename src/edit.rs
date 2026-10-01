// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Persistent source-bound edit stages and explicit candidate operations.

mod editor;
mod error;
mod fields;
mod options;
mod report;

pub(crate) use error::EditError;
use error::Kind;
pub(crate) use options::Options;

use crate::output_cli::{self, HumanLevel, ReportFormat};
use crate::spec::{ParsedSpec, candidate, document::Snapshot};
use crate::{file_output, stage, utf8_file, workspace};
use fs_err as fs;
use std::{
    fmt::Write as _,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};
use toml::Table;

struct Edit {
    path: PathBuf,
    snapshot: Snapshot<'static>,
    baseline: Option<crate::check_report::CheckReport>,
    draft: Option<PathBuf>,
    stage_dir: PathBuf,
    subject: PathBuf,
    // Retain the cooperative WORK lock throughout staging and publication.
    development: Option<workspace::Development>,
    committed_main: bool,
    expand_stage: bool,
    candidate: Option<Result<candidate::Candidate, EditError>>,
    cache: Option<Cached>,
    source_hashes: Option<crate::source::SourceHashes>,
}

impl Edit {
    fn result(&self) -> Result<&candidate::Candidate, &EditError> {
        self.candidate
            .as_ref()
            .expect("candidate attempted")
            .as_ref()
    }

    fn assign(&mut self, options: &Options, inline: bool) -> Result<(), EditError> {
        let path = self.draft.as_ref().expect("editing creates a stage");
        let mut edited = stage::read_document_path(path)?;
        fields::assign(&mut edited, &options.set).map_err(|error| {
            EditError::at(
                Kind::InvalidAssignment,
                &self.path,
                self.snapshot.selection(),
                error,
            )
        })?;
        if inline {
            fields::edit_inline(&mut edited, &options.fields)?;
        }
        stage::save_document(path, &edited)?;
        self.select_edit_input()?;
        Ok(())
    }

    fn cache_candidate(&mut self, diff: bool, show_diff: bool) -> Result<(), EditError> {
        if let Some(Ok(candidate)) = &self.candidate {
            let stem = self
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or("SPEC stem must be UTF-8")?;
            let path = stage::cache(&self.stage_dir, stem, candidate.spec.source())?;
            let diff = if diff {
                let text =
                    stage::diff(&self.path, self.snapshot.source(), candidate.spec.source())?;
                let diff_path = stage::save_diff(&self.stage_dir, stem, &text)?;
                if show_diff {
                    io::stdout()
                        .lock()
                        .write_all(text.as_bytes())
                        .map_err(|error| format!("failed to write output to stdout: {error}"))?;
                }
                Some((diff_path, text))
            } else {
                None
            };
            self.cache = Some(Cached { path, diff });
        }
        Ok(())
    }

    fn select_edit_input(&mut self) -> Result<(), EditError> {
        if let Some(area) = &mut self.development {
            // create/load return canonical draft paths. External prepared stages
            // are not the WORK input consumed by gen.
            if self.draft.as_deref().and_then(Path::parent)
                != Some(area.directory().join("stage").as_path())
            {
                return Ok(());
            }
            area.select_input(workspace::GenerationInput::Edit)
                .map_err(|error| {
                    EditError::at(
                        Kind::OperationFailed,
                        &self.subject,
                        self.snapshot.selection(),
                        format!(
                            "stage saved, but cannot select it as the generation input: {error}"
                        ),
                    )
                })?;
        }
        Ok(())
    }

    fn ensure_unchanged(&self) -> Result<(), EditError> {
        if self.committed_main {
            return Ok(());
        }
        if !utf8_file::is_unchanged(&self.path, self.snapshot.source()).map_err(|error| {
            EditError::at(
                Kind::InputRead,
                &self.path,
                self.snapshot.selection(),
                error.to_string(),
            )
        })? {
            return Err(EditError::at(
                Kind::SourceChanged,
                &self.path,
                self.snapshot.selection(),
                format!(
                    "{}: source changed; prepare a fresh stage",
                    self.path.display()
                ),
            ));
        }
        Ok(())
    }
}

pub(crate) fn run(mut options: Options) -> Result<bool, EditError> {
    if options.menu && options.fields.is_empty() {
        let field = output_cli::select_edit_field()?
            .ok_or("--menu requires a terminal; use --set FIELD=VALUE for CLI-only editing")?;
        options.fields.push(field.to_owned());
    }
    let result = execute(&options);
    if !matches!(options.format, Some(ReportFormat::Toml)) {
        return result.and_then(|result| {
            let success = result.is_success();
            result.publication.map(|_| success)
        });
    }
    let success = result.as_ref().is_ok_and(EditResult::is_success);
    let envelope = report::envelope(&result, &options);
    crate::report::write(&mut io::stdout().lock(), &envelope).map_err(|error| {
        let mut message = format!("failed to write output to stdout: {error}");
        let written = match &result {
            Ok(result) => match &result.publication {
                Ok(outcomes) => outcomes
                    .iter()
                    .filter_map(|outcome| match outcome {
                        file_output::EditOutcome::Written(path) => Some(path),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                Err(error) => error.written_paths().iter().collect(),
            },
            Err(error) => error.written_paths().iter().collect(),
        };
        if !written.is_empty() {
            write!(
                message,
                "\nFiles already written: {written:?}; publication was not rolled back."
            )
            .expect("String formatting cannot fail");
        }
        EditError::from(message)
    })?;
    Ok(success)
}

fn input(
    path: PathBuf,
    parsed: &ParsedSpec<'_>,
    fields: &[String],
    safe: bool,
    stage_dir: PathBuf,
    draft: Option<PathBuf>,
    checked: bool,
) -> Result<Edit, EditError> {
    let snapshot = if safe {
        Snapshot::capture_supported(parsed)
    } else {
        Snapshot::capture_selected(parsed, fields)
    }
    .map_err(|error| {
        EditError::at(
            Kind::UnmappableFields,
            &path,
            fields,
            format!("{}: {error}", path.display()),
        )
    })?
    .into_owned();
    let baseline =
        checked.then(|| crate::check::analyze(parsed, crate::check::Policy::Authoring, &[]));
    Ok(Edit {
        subject: path.clone(),
        path,
        snapshot,
        baseline,
        draft,
        stage_dir,
        development: None,
        committed_main: false,
        expand_stage: false,
        candidate: None,
        cache: None,
        source_hashes: None,
    })
}

fn load_inputs(options: &Options) -> Result<Vec<Edit>, EditError> {
    if options.pkgname.is_some() && options.works.len() != 1 {
        return Err("--pkgname requires exactly one development area".into());
    }
    let checked = options.check || options.apply;
    let inputs = if let Some(dir) = &options.from {
        stage::load(dir)?
            .into_iter()
            .map(|draft| {
                let stage_dir = draft
                    .path
                    .parent()
                    .expect("stage file has a parent")
                    .to_owned();
                input(
                    draft.source,
                    &ParsedSpec::parse(draft.original),
                    &draft.fields,
                    false,
                    stage_dir,
                    Some(draft.path),
                    checked,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let mut fields = options.fields.clone();
        fields.extend(options.set.iter().map(|(field, _)| field.clone()));
        for number in &options.hash_sources {
            let field = format!("sources.{number}.sha256");
            if !fields.contains(&field) {
                fields.push(field);
            }
        }
        let sources = options.specs.iter().map(|path| (path.clone(), None)).chain(
            options
                .works
                .iter()
                .map(|work| (PathBuf::from(work), Some(work.as_str()))),
        );
        sources
            .map(|(path, work)| load_input(options, &path, work, &fields))
            .collect::<Result<Vec<_>, _>>()?
    };
    validate_inputs(&inputs, options)?;
    Ok(inputs)
}

fn validate_inputs(inputs: &[Edit], options: &Options) -> Result<(), EditError> {
    if inputs.is_empty() {
        return Err("no SPEC files selected".into());
    }
    if inputs.len() != 1
        && (options.stdout || options.output.is_some() || options.expect_sha256.is_some())
    {
        return Err("--stdout, --output and --expect-sha256 require exactly one SPEC".into());
    }
    if let Some(expected) = &options.expect_sha256 {
        let item = &inputs[0];
        let actual = utf8_file::sha256(item.snapshot.source());
        if &actual != expected {
            return Err(EditError::at(
                Kind::SourceChanged,
                &item.path,
                item.snapshot.selection(),
                format!(
                    "{}: expected SHA-256 {expected}, found {actual}; no SPEC edits published",
                    item.path.display()
                ),
            ));
        }
    }
    for (index, item) in inputs.iter().enumerate() {
        if inputs[..index].iter().any(|other| other.path == item.path) {
            return Err(format!("{}: SPEC selected more than once", item.path.display()).into());
        }
    }
    // Reject destructive destinations before saving a new scope or selecting an input.
    if let Some(output) = &options.output {
        for item in inputs {
            if let Some(area) = &item.development {
                area.protect_output(output).map_err(|error| {
                    EditError::at(
                        Kind::OperationFailed,
                        output,
                        item.snapshot.selection(),
                        error.to_string(),
                    )
                })?;
            }
            stage::protect_output(output, item.draft.as_deref())?;
        }
    }
    Ok(())
}

fn load_input(
    options: &Options,
    path: &Path,
    work: Option<&str>,
    fields: &[String],
) -> Result<Edit, EditError> {
    let io_error =
        |error: io::Error| EditError::at(Kind::InputRead, path, fields, error.to_string());
    let (requested, source, stage_dir, development, committed_main) = if let Some(work) = work {
        let workspace = workspace::discover().map_err(io_error)?;
        let mut area = workspace
            .development(work, options.pkgname.as_deref(), false)
            .map_err(io_error)?;
        let (requested, source, committed_main) = if options.checks_only() {
            let (requested, source, revision) = area.source().map_err(io_error)?;
            (requested, source, revision.is_some())
        } else {
            area.create().map_err(io_error)?;
            let requested = area.spec().map_err(io_error)?;
            let source = utf8_file::read(&requested).map_err(|error| {
                EditError::at(Kind::InputRead, &requested, fields, error.to_string())
            })?;
            (requested, source, false)
        };
        let stage_dir = options
            .prepare
            .clone()
            .unwrap_or_else(|| area.directory().join("stage"));
        (requested, source, stage_dir, Some(area), committed_main)
    } else {
        let requested = fs::canonicalize(path).map_err(io_error)?;
        let source = utf8_file::read(&requested).map_err(|error| {
            EditError::at(Kind::InputRead, &requested, fields, error.to_string())
        })?;
        let stem = requested.file_stem().ok_or("SPEC file has no stem")?;
        let stage_dir = options.prepare.clone().unwrap_or_else(|| {
            requested
                .parent()
                .expect("canonical path has parent")
                .join(".ruyipack-stage")
                .join(stem)
        });
        (requested, source, stage_dir, None, false)
    };
    let parsed = ParsedSpec::parse(source);
    let safe = !options.all && fields.is_empty();
    let mut selected = fields.to_vec();
    if options.hash {
        // Field discovery proves replacement locations, not that old URLs can be downloaded.
        let editable = Snapshot::capture_supported(&parsed)
            .map_err(|error| EditError::at(Kind::UnmappableFields, &requested, &selected, error))?;
        if let Some(sources) = editable
            .document()
            .get("sources")
            .and_then(|value| value.as_table())
        {
            for number in sources.keys() {
                let field = format!("sources.{number}.sha256");
                if !selected.contains(&field) {
                    selected.push(field);
                }
            }
        }
    }
    // Resume existing values instead of replacing unfinished editor work.
    let saved = if options.prepare.is_none() && stage_dir.join(".state/index.toml").exists() {
        stage::load(&stage_dir)?
            .into_iter()
            .find(|draft| draft.source == requested)
    } else {
        None
    };
    let (selected, draft, safe, expand_stage) = if let Some(draft) = saved {
        if draft.original != parsed.source() {
            return Err(EditError::at(
                Kind::SourceChanged,
                &requested,
                &draft.fields,
                format!(
                    "{}: stage baseline no longer matches the source; prepare a fresh stage",
                    requested.display()
                ),
            ));
        }
        let previous = draft.fields.clone();
        let mut union = draft.fields;
        if options.all {
            // --all is an explicit full mapping, even when a narrow stage exists.
            union.clear();
        } else if safe && (options.editor.is_some() || !options.generates_candidate()) {
            // Reopen unfinished selected TOML before trying to expand
            // its scope. An explicit new selection still requires repair.
            let pending = stage::read_text(&draft.path)?;
            if toml::from_str::<Table>(&pending).is_ok() {
                union = Snapshot::capture_supported(&parsed)
                    .map_err(EditError::from)?
                    .selection()
                    .to_vec();
            }
        } else {
            for field in selected {
                if !union.contains(&field) {
                    union.push(field);
                }
            }
        }
        let expand = union != previous;
        (union, Some(draft.path), false, expand)
    } else {
        (selected, None, safe, false)
    };
    let mut item = input(
        requested,
        &parsed,
        &selected,
        safe,
        stage_dir,
        draft,
        options.check || options.apply,
    )?;
    item.development = development;
    item.committed_main = committed_main;
    item.expand_stage = expand_stage;
    item.subject = work.map_or_else(|| item.path.clone(), PathBuf::from);
    Ok(item)
}

fn create_stages(inputs: &mut [Edit]) -> Result<(), EditError> {
    // Existing selected stages may need more fields in their immutable mapping.
    for item in inputs.iter_mut() {
        if let Some(path) = &item.draft
            && item.expand_stage
        {
            let mut document = stage::read_document_path(path).map_err(|error| {
                EditError::at(
                    Kind::InvalidDraft,
                    path,
                    item.snapshot.selection(),
                    format!("{error}; repair saved TOML before expanding the field selection"),
                )
            })?;
            stage::complete_missing(&mut document, item.snapshot.document());
            stage::update(path, item.snapshot.selection(), &document, None)?;
        }
    }
    for index in 0..inputs.len() {
        if inputs[index].draft.is_some() {
            continue;
        }
        let group = (index..inputs.len())
            .filter(|&other| {
                inputs[other].draft.is_none() && inputs[other].stage_dir == inputs[index].stage_dir
            })
            .collect::<Vec<_>>();
        if group.len() > 1 {
            let dir = &inputs[index].stage_dir;
            let resolved = fs::canonicalize(dir).ok().or_else(|| {
                let parent = dir
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                fs::canonicalize(parent)
                    .ok()
                    .zip(dir.file_name())
                    .map(|(parent, name)| parent.join(name))
            });
            if group.iter().any(|&index| {
                inputs[index]
                    .development
                    .as_ref()
                    .is_some_and(|area| resolved.as_ref() == Some(&area.directory().join("stage")))
            }) {
                return Err("a WORK stage must contain exactly its bound package; prepare this batch in an external DIR".into());
            }
        }
        let sources = group
            .iter()
            .map(|&index| {
                let item = &inputs[index];
                (
                    item.path.as_path(),
                    item.snapshot.source(),
                    item.snapshot.selection(),
                    item.snapshot.document(),
                )
            })
            .collect::<Vec<_>>();
        let paths = stage::create(&inputs[index].stage_dir, &sources)?;
        for (index, path) in group.into_iter().zip(paths) {
            inputs[index].draft = Some(path);
        }
    }
    Ok(())
}

fn execute(options: &Options) -> Result<EditResult, EditError> {
    let inline = options.menu
        || (!options.fields.is_empty() && options.editor.is_none() && options.prepare.is_none());
    if inline {
        if matches!(options.format, Some(ReportFormat::Toml)) {
            return Err("interactive field editing cannot produce a TOML report; use --set FIELD=VALUE, or --prepare DIR to save selected TOML fields".into());
        }
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err("inline field editing requires a terminal; use --set FIELD=VALUE or --editor COMMAND".into());
        }
    }
    let mut inputs = load_inputs(options)?;
    let pure_check = options.checks_only();
    let resumes = inputs.iter().all(|item| item.draft.is_some());
    let requested_operation = options.generates_candidate();
    let opens_editor = options.editor.is_some()
        || (options.from.is_none()
            && options.prepare.is_none()
            && options.set.is_empty()
            && !inline
            && !options.hash
            && options.hash_sources.is_empty()
            && !pure_check
            && !(resumes && requested_operation && options.fields.is_empty() && !options.all));
    if opens_editor && matches!(options.format, Some(ReportFormat::Toml)) {
        return Err("TOML reports require an explicit non-interactive action: --set, --hash, --prepare, --from, or --check".into());
    }
    if !pure_check {
        create_stages(&mut inputs)?;
        if options.prepare.is_some() {
            for item in &mut inputs {
                item.select_edit_input()?;
            }
        }
    }
    if opens_editor {
        let paths = inputs
            .iter()
            .filter_map(|item| item.draft.as_deref())
            .collect::<Vec<_>>();
        editor::open(&paths, options.editor.as_deref()).map_err(|error| {
            EditError::from(format!(
                "{error}\nPersistent stage retained: {}",
                inputs[0].stage_dir.display()
            ))
        })?;
        for item in &mut inputs {
            item.select_edit_input()?;
        }
    }
    if inline || !options.set.is_empty() {
        for item in &mut inputs {
            item.assign(options, inline)?;
        }
    }
    if !matches!(options.format, Some(ReportFormat::Toml)) {
        for item in &inputs {
            if let Some(path) = &item.draft {
                output_cli::stderr()
                    .message(
                        HumanLevel::Info,
                        Some(&item.subject),
                        format_args!("stage saved: {}", output_cli::human_path(path).display()),
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    if !requested_operation {
        // A closed editor may leave syntactically unfinished TOML. It is saved work,
        // not evidence of a constructed or statically valid candidate.
        return Ok(EditResult {
            inputs,
            publication: Ok(Vec::new()),
        });
    }
    for item in &mut inputs {
        item.candidate = Some(read_candidate(item, options));
    }
    if options.diff || options.stdout || options.apply {
        for item in &mut inputs {
            item.cache_candidate(
                options.diff,
                !matches!(options.format, Some(ReportFormat::Toml)),
            )?;
        }
    }
    let publication = publish(options, &inputs);
    Ok(EditResult {
        inputs,
        publication,
    })
}

fn read_candidate(item: &mut Edit, options: &Options) -> Result<candidate::Candidate, EditError> {
    item.ensure_unchanged()?;
    let mut document = if let Some(path) = &item.draft {
        stage::read_document_path(path).map_err(|error| {
            EditError::at(Kind::InvalidDraft, path, item.snapshot.selection(), error)
        })?
    } else {
        item.snapshot.document().clone()
    };
    if options.hash || !options.hash_sources.is_empty() {
        complete_hashes(item, &mut document, options)?;
    }
    candidate::prepare(
        &item.snapshot,
        &document,
        &options.defines,
        options.check || options.apply,
    )
    .map_err(|error| {
        EditError::at(
            Kind::InvalidCandidate,
            item.draft.as_deref().unwrap_or(&item.path),
            item.snapshot.selection(),
            error,
        )
    })
}

fn complete_hashes(
    item: &mut Edit,
    document: &mut Table,
    options: &Options,
) -> Result<(), EditError> {
    let pending = item
        .snapshot
        .render_before_hashing(document, &options.defines)
        .map_err(|error| {
            EditError::at(
                Kind::SourceHashFailed,
                &item.path,
                item.snapshot.selection(),
                error,
            )
        })?;
    let resolved = crate::spec::sources::resolve(&pending, &options.defines).map_err(|error| {
        EditError::source_hash(
            crate::source::Error::resolution(error),
            &item.path,
            item.snapshot.selection(),
        )
    })?;
    let urls = if options.hash {
        crate::source::prepare_remote(&resolved)
    } else {
        crate::source::prepare_selected(&resolved, &options.hash_sources)
    }
    .map_err(|error| EditError::source_hash(error, &item.path, item.snapshot.selection()))?;
    for number in urls.keys() {
        let field = format!("sources.{number}.sha256");
        if options.set.iter().any(|(key, _)| key == &field) {
            return Err(EditError::at(
                Kind::SourceHashFailed,
                &item.path,
                item.snapshot.selection(),
                format!("{field}: choose either --set or --hash, not both"),
            ));
        }
        if crate::spec::document::table::lookup(document, &field).is_none() {
            return Err(EditError::at(
                Kind::SourceHashFailed,
                &item.path,
                item.snapshot.selection(),
                format!("{field}: stage does not include this digest field"),
            ));
        }
    }
    let hashes = crate::source::SourceHashes {
        input_sha256: utf8_file::sha256(pending.source()),
        defines: options.defines.clone(),
        sources: crate::source::download_prepared(urls).map_err(|error| {
            EditError::source_hash(error, &item.path, item.snapshot.selection())
        })?,
    };
    item.ensure_unchanged()?;
    stage::complete_digests(document, &hashes.sources)?;
    if let Some(path) = &item.draft {
        stage::save_document(path, document)?;
        item.select_edit_input()?;
    }
    item.source_hashes = Some(hashes);
    Ok(())
}

fn static_check_error(item: &Edit, candidate: &candidate::Candidate) -> Option<EditError> {
    let report = candidate.report.as_ref()?;
    let baseline = item.baseline.as_ref().expect("checking captures baseline");
    (!report.allows_edit(baseline)).then(|| EditError::at(Kind::StaticCheckFailed,
        &item.path, item.snapshot.selection(), format!("{}: candidate failed static checks: {}. No edited SPECs published; repair the reported fields and retry", item.path.display(),
            match report.introduced_static_blockers(baseline) {
                Some(true) => "new or changed static blockers (compare baseline_report in --check --format toml)",
                Some(false) => "pre-existing blockers cannot be confirmed as unchanged rule inputs",
                None => "static checks remain incomplete; unresolved results cannot be attributed to this edit",
            })))
}

fn error_message(error: &EditError, options: &Options) -> String {
    if matches!(options.format, Some(ReportFormat::Toml)) {
        error.message.clone()
    } else {
        error.to_string()
    }
}

fn publish(options: &Options, inputs: &[Edit]) -> Result<Vec<file_output::EditOutcome>, EditError> {
    if !matches!(options.format, Some(ReportFormat::Toml)) {
        report::write_candidates(inputs, options.check).map_err(|error| error.to_string())?;
    }
    let construction_errors = inputs
        .iter()
        .filter_map(|item| {
            item.result()
                .err()
                .map(|error| error_message(error, options))
        })
        .collect::<Vec<_>>();
    if !construction_errors.is_empty() {
        return Err(construction_errors.join("\n").into());
    }
    let static_errors = inputs
        .iter()
        .filter_map(|item| static_check_error(item, item.result().expect("candidate constructed")))
        .collect::<Vec<_>>();
    if (options.check || options.apply) && !static_errors.is_empty() {
        if !options.apply && !matches!(options.format, Some(ReportFormat::Toml)) {
            for error in &static_errors {
                output_cli::stderr()
                    .message(HumanLevel::Error, None, format_args!("{error}"))
                    .map_err(|error| error.to_string())?;
            }
        }
        if options.apply {
            return Err(static_errors
                .iter()
                .map(|error| error_message(error, options))
                .collect::<Vec<_>>()
                .join("\n")
                .into());
        }
    }
    if options.stdout {
        io::stdout()
            .lock()
            .write_all(
                inputs[0]
                    .result()
                    .expect("candidate constructed")
                    .spec
                    .source()
                    .as_bytes(),
            )
            .map_err(|error| format!("failed to write output to stdout: {error}"))?;
    }
    if !options.apply {
        return Ok(Vec::new());
    }
    for item in inputs {
        if let Some(output) = &options.output {
            let parent = output
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::canonicalize(parent).map_err(|source| {
                EditError::publication(file_output::OutputError::Read {
                    path: parent.to_owned(),
                    source,
                })
            })?;
            stage::protect_output(output, item.draft.as_deref())?;
        }
    }
    let files = inputs
        .iter()
        .map(|item| file_output::EditFile {
            source_path: &item.path,
            original: item.snapshot.source(),
            contents: item.result().expect("candidate constructed").spec.source(),
        })
        .collect::<Vec<_>>();
    let outcomes = file_output::run_edits(
        &mut io::stdout().lock(),
        &files,
        options.output.as_deref(),
        |path| {
            if options.force {
                Ok(file_output::ConflictAction::Overwrite)
            } else {
                output_cli::select_edit_action(path)
            }
        },
    )
    .map_err(EditError::publication)?;
    rebase_stages(inputs, &outcomes)?;
    if !matches!(options.format, Some(ReportFormat::Toml)) {
        for outcome in &outcomes {
            output_cli::write_outcome(&mut output_cli::stderr(), outcome)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(outcomes)
}

fn rebase_stages(inputs: &[Edit], outcomes: &[file_output::EditOutcome]) -> Result<(), EditError> {
    // Only publication to the bound source advances its recovery baseline.
    // A copy output leaves the original binding and its pending edit intact.
    for item in inputs {
        if outcomes.iter().any(|outcome| matches!(outcome,
            file_output::EditOutcome::Written(path) | file_output::EditOutcome::Unchanged(path) if path == &item.path))
            && let Some(path) = &item.draft
        {
            let candidate = item.result().expect("published candidate");
            let document = Snapshot::capture_selected(&candidate.spec, item.snapshot.selection())
                .map_err(EditError::from)?;
            stage::update(path, item.snapshot.selection(), document.document(), Some(candidate.spec.source()))
                .map_err(|error| EditError::publication(file_output::OutputError::Partial {
                    written: outcomes.iter().filter_map(|outcome| match outcome {
                        file_output::EditOutcome::Written(path) => Some(path.clone()), _ => None,
                    }).collect(),
                    source: Box::new(file_output::OutputError::Write { path: path.clone(), source: io::Error::other(error) }),
                }))?;
        }
    }
    Ok(())
}

struct Cached {
    path: PathBuf,
    diff: Option<(PathBuf, String)>,
}

struct EditResult {
    inputs: Vec<Edit>,
    publication: Result<Vec<file_output::EditOutcome>, EditError>,
}

impl EditResult {
    fn is_success(&self) -> bool {
        self.publication.is_ok()
            && self.inputs.iter().all(|item| {
                item.candidate.as_ref().is_none_or(|check|
                matches!(check, Ok(candidate) if static_check_error(item, candidate).is_none()))
            })
    }
    fn static_valid(&self) -> Option<bool> {
        let mut valid = true;
        for item in &self.inputs {
            valid &= item
                .candidate
                .as_ref()?
                .as_ref()
                .ok()?
                .report
                .as_ref()?
                .is_success();
        }
        Some(valid)
    }
}
