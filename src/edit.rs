// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source-bound editable documents and explicit candidate operations.

mod error;
mod fields;
mod options;
mod report;

pub(crate) use error::EditError;
use error::Kind;
use options::Interaction;
pub(crate) use options::Options;

use crate::output_cli::{self, HumanLevel, ReportFormat};
use crate::spec::{ParsedSpec, candidate, document::Snapshot};
use crate::{draft, file_output, utf8_file, workspace};
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
    authoring: bool,
    created_work: bool,
    draft_dir: PathBuf,
    subject: PathBuf,
    // Retain the cooperative WORK lock throughout staging and publication.
    development: Option<workspace::Development>,
    committed_main: bool,
    expand_fields: bool,
    input_changed: Option<bool>,
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

    fn document(&self) -> Result<Table, EditError> {
        let Some(path) = &self.draft else {
            return Ok(self.snapshot.document().clone());
        };
        let document = draft::read_document_path(path).map_err(|error| {
            EditError::at(Kind::InvalidDraft, path, self.snapshot.selection(), error)
        })?;
        if !self.authoring {
            return Ok(document);
        }
        let mut selected = self.snapshot.document().clone();
        fields::overlay(&mut selected, &document);
        Ok(selected)
    }

    fn save_document(&self, path: &Path, values: &Table) -> Result<(), EditError> {
        if self.authoring {
            let mut document = draft::read_document_path(path)?;
            fields::update_authoring(&mut document, &self.document()?, values)?;
            crate::render::manifest::validate_structure(&document)
                .map_err(|error| EditError::from(error.to_string()))?;
            draft::save_document(path, &document)?;
        } else if self.expand_fields {
            draft::update(path, self.snapshot.selection(), values, None)?;
        } else {
            draft::save_document(path, values)?;
        }
        Ok(())
    }

    fn assign(&mut self, options: &Options, inline: bool) -> Result<(), EditError> {
        let mut edited = self.document()?;
        if self.expand_fields {
            draft::complete_missing(&mut edited, self.snapshot.document());
        }
        let original = edited.clone();
        fields::assign(&mut edited, &options.set, &options.add).map_err(|error| {
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
        self.input_changed = Some(edited != original);
        if edited != original {
            self.ensure_unchanged()?;
            if let Some(path) = &self.draft {
                self.save_document(path, &edited)?;
            } else {
                self.draft = draft::create(
                    &self.draft_dir,
                    &[draft::Input {
                        path: &self.path,
                        destination: None,
                        original: self.snapshot.source(),
                        fields: self.snapshot.selection(),
                        values: &edited,
                    }],
                )?
                .pop();
            }
        }
        Ok(())
    }

    fn artifact_directory(&self) -> PathBuf {
        self.development.as_ref().map_or_else(
            || self.draft_dir.clone(),
            |area| area.directory().join(".cache"),
        )
    }

    fn cache_candidate(&mut self, diff: bool, show_diff: bool) -> Result<(), EditError> {
        if let Some(Ok(candidate)) = &self.candidate {
            if candidate.spec.source() == self.snapshot.source() {
                return Ok(());
            }
            let stem = self
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or("SPEC stem must be UTF-8")?;
            let path = draft::cache(&self.artifact_directory(), stem, candidate.spec.source())?;
            let diff = if diff {
                let text =
                    draft::diff(&self.path, self.snapshot.source(), candidate.spec.source())?;
                let diff_path = draft::save_diff(&self.artifact_directory(), stem, &text)?;
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
                    "{}: source changed; prepare a fresh draft",
                    self.path.display()
                ),
            ));
        }
        Ok(())
    }
}

pub(crate) struct Operation {
    options: Options,
    result: Result<EditResult, EditError>,
}

impl serde::Serialize for Operation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&report::envelope(&self.result, &self.options), serializer)
    }
}

pub(crate) fn evaluate(mut options: Options) -> Operation {
    let result = execute(&mut options);
    Operation { options, result }
}

pub(crate) fn run(options: Options) -> Result<bool, EditError> {
    evaluate(options).print()
}

impl Operation {
    pub(crate) fn success(&self) -> bool {
        self.result
            .as_ref()
            .is_ok_and(|result| result.success_for(&self.options))
    }

    pub(crate) fn print(self) -> Result<bool, EditError> {
        let Self { options, result } = self;
        if !matches!(options.format, Some(ReportFormat::Toml)) {
            return result.and_then(|result| {
                let success = result.success_for(&options);
                result.publication.map(|_| success)
            });
        }
        let success = result
            .as_ref()
            .is_ok_and(|result| result.success_for(&options));
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
}

fn input(
    path: PathBuf,
    parsed: &ParsedSpec<'_>,
    fields: &[String],
    safe: bool,
    draft_dir: PathBuf,
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
        authoring: false,
        created_work: false,
        draft_dir,
        development: None,
        committed_main: false,
        expand_fields: false,
        input_changed: None,
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
        draft::load(dir)?
            .into_iter()
            .map(|draft| {
                let draft_dir = draft
                    .path
                    .parent()
                    .expect("draft file has a parent")
                    .to_owned();
                input(
                    draft.source,
                    &ParsedSpec::parse(draft.original),
                    &draft.fields,
                    false,
                    draft_dir,
                    Some(draft.path),
                    checked,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let mut fields = options.fields.clone();
        fields.extend(
            options
                .set
                .iter()
                .chain(&options.add)
                .map(|(field, _)| field.clone()),
        );
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
            if !item.authoring {
                draft::protect_output(output, item.draft.as_deref())?;
            }
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
    let mut created_work = false;
    let (requested, source, draft_dir, development, committed_main) = if let Some(work) = work {
        let workspace = workspace::discover().map_err(io_error)?;
        let mut area = workspace
            .development(work, options.pkgname.as_deref(), false)
            .map_err(io_error)?;
        let (requested, source, committed_main) = if options.checks_only() {
            let (requested, source, revision) = area.source().map_err(io_error)?;
            (requested, source, revision.is_some())
        } else {
            created_work = !area.directory().join("recipe").exists();
            area.create().map_err(io_error)?;
            if created_work && !matches!(options.format, Some(ReportFormat::Toml)) {
                output_cli::stderr()
                    .message(
                        HumanLevel::Info,
                        Some(Path::new(work)),
                        format_args!(
                            "created: {}",
                            output_cli::human_path(area.directory()).display()
                        ),
                    )
                    .map_err(|error| EditError::from(error.to_string()))?;
            }
            let requested = area.spec().map_err(io_error)?;
            let source = utf8_file::read(&requested).map_err(|error| {
                EditError::at(Kind::InputRead, &requested, fields, error.to_string())
            })?;
            (requested, source, false)
        };
        let draft_dir = options
            .prepare
            .clone()
            .unwrap_or_else(|| area.directory().to_path_buf());
        (requested, source, draft_dir, Some(area), committed_main)
    } else {
        let requested = fs::canonicalize(path).map_err(io_error)?;
        let source = utf8_file::read(&requested).map_err(|error| {
            EditError::at(Kind::InputRead, &requested, fields, error.to_string())
        })?;
        let stem = requested.file_stem().ok_or("SPEC file has no stem")?;
        let draft_dir = options.prepare.clone().unwrap_or_else(|| {
            requested
                .parent()
                .expect("canonical path has parent")
                .join(".ruyipack-draft")
                .join(stem)
        });
        (requested, source, draft_dir, None, false)
    };
    let parsed = ParsedSpec::parse(source);
    let safe = !options.all && fields.is_empty();
    let mut selected = fields.to_vec();
    if options.hash {
        // Field discovery proves replacement locations, not that old URLs can be downloaded.
        let editable = Snapshot::capture_supported(&parsed)
            .map_err(|error| EditError::at(Kind::UnmappableFields, &requested, &selected, error))?;
        if options.repair_missing
            && editable
                .document()
                .get("package")
                .and_then(|value| value.get("summary"))
                .and_then(toml::Value::as_str)
                .and_then(crate::check::metadata::fixed_summary)
                .is_some()
        {
            selected.push("package.summary".into());
        }
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
    let saved = if options.prepare.is_none() && draft_dir.join(".state/index.toml").exists() {
        draft::load(&draft_dir)?
            .into_iter()
            .find(|draft| draft.source == requested)
    } else {
        None
    };
    let authoring = development
        .as_ref()
        .is_some_and(|area| !area.spec_authoring())
        && options.prepare.is_none();
    let (selected, draft, safe, expand_fields) = if let Some(draft) = saved {
        if draft.original != parsed.source() {
            return Err(EditError::at(
                Kind::SourceChanged,
                &requested,
                &draft.fields,
                format!(
                    "{}: draft baseline no longer matches the source; prepare a fresh draft",
                    requested.display()
                ),
            ));
        }
        let previous = draft.fields.clone();
        let mut union = draft.fields;
        if options.all {
            // --all is an explicit full mapping, even when a narrow draft exists.
            union.clear();
        } else if safe
            && !options.menu
            && (options.editor.is_some() || !options.generates_candidate())
        {
            // Reopen unfinished selected TOML before trying to expand
            // its scope. An explicit new selection still requires repair.
            let pending = draft::read_text(&draft.path)?;
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
        let path = authoring.then(|| development.as_ref().expect("WORK").manifest());
        (selected, path, safe, false)
    };
    let mut item = input(
        requested,
        &parsed,
        &selected,
        safe,
        draft_dir,
        draft,
        options.check || options.apply,
    )?;
    item.authoring = authoring;
    item.created_work = created_work;
    item.development = development;
    item.committed_main = committed_main;
    item.expand_fields = expand_fields;
    item.subject = work.map_or_else(|| item.path.clone(), PathBuf::from);
    Ok(item)
}

fn create_drafts(inputs: &mut [Edit]) -> Result<(), EditError> {
    // Existing selected drafts may need more fields in their immutable mapping.
    for item in inputs.iter_mut() {
        if let Some(path) = &item.draft
            && item.expand_fields
        {
            let mut document = draft::read_document_path(path).map_err(|error| {
                EditError::at(
                    Kind::InvalidDraft,
                    path,
                    item.snapshot.selection(),
                    format!("{error}; repair saved TOML before expanding the field selection"),
                )
            })?;
            draft::complete_missing(&mut document, item.snapshot.document());
            draft::update(path, item.snapshot.selection(), &document, None)?;
        }
    }
    for index in 0..inputs.len() {
        if inputs[index].draft.is_some() {
            continue;
        }
        let group = (index..inputs.len())
            .filter(|&other| {
                inputs[other].draft.is_none() && inputs[other].draft_dir == inputs[index].draft_dir
            })
            .collect::<Vec<_>>();
        if group.len() > 1 {
            let dir = &inputs[index].draft_dir;
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
                    .is_some_and(|area| resolved.as_ref() == Some(&area.directory().to_path_buf()))
            }) {
                return Err("a WORK draft must contain exactly its bound package; prepare this batch in an external DIR".into());
            }
        }
        let sources = group
            .iter()
            .map(|&index| {
                let item = &inputs[index];
                draft::Input {
                    path: &item.path,
                    destination: None,
                    original: item.snapshot.source(),
                    fields: item.snapshot.selection(),
                    values: item.snapshot.document(),
                }
            })
            .collect::<Vec<_>>();
        let paths = draft::create(&inputs[index].draft_dir, &sources)?;
        for (index, path) in group.into_iter().zip(paths) {
            inputs[index].draft = Some(path);
        }
    }
    Ok(())
}

fn execute(options: &mut Options) -> Result<EditResult, EditError> {
    let interaction = options.interaction();
    let inline = interaction == Interaction::Inline;
    if interaction == Interaction::Editor && matches!(options.format, Some(ReportFormat::Toml)) {
        return Err("TOML reports require an explicit non-interactive action: --set, --hash, --prepare, --from, or --check".into());
    }
    if inline {
        if matches!(options.format, Some(ReportFormat::Toml)) {
            return Err("interactive field editing cannot produce a TOML report; use --set FIELD=VALUE, or --prepare DIR to save selected TOML fields".into());
        }
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err("inline field editing requires a terminal; use --set FIELD=VALUE or --editor COMMAND".into());
        }
    }
    let mut inputs = load_inputs(options)?;
    if options.repair_missing {
        for item in &inputs {
            if item.authoring {
                let path = item.draft.as_ref().expect("authoring path");
                let manifest =
                    crate::render::manifest::parse_document(draft::read_document_path(path)?)
                        .map_err(|error| EditError::from(error.to_string()))?;
                let candidate = crate::render::run(&manifest)
                    .map_err(|error| EditError::from(error.to_string()))?;
                if candidate.source() != item.snapshot.source() {
                    return Err(
                        "WORK has pending TOML edits; apply or resolve them before auto-fix".into(),
                    );
                }
            }
            let mut document = item.document()?;
            draft::complete_missing(&mut document, item.snapshot.document());
            if document != *item.snapshot.document() {
                return Err(
                    "WORK has pending TOML edits; apply or resolve them before auto-fix".into(),
                );
            }
        }
    }
    if options.menu && options.fields.is_empty() {
        let [item] = inputs.as_mut_slice() else {
            return Err("--menu edits one SPEC at a time; use --set for multiple inputs".into());
        };
        let parsed = ParsedSpec::parse(item.snapshot.source());
        let available = Snapshot::capture_supported(&parsed).map_err(EditError::from)?;
        let field = output_cli::select_edit_field(&available)?
            .ok_or("--menu requires a terminal; use --set FIELD=VALUE")?;
        let mut selected = if item.draft.is_some() && !item.authoring {
            item.snapshot.selection().to_vec()
        } else {
            vec![]
        };
        if !selected.contains(&field) {
            selected.push(field.clone());
        }
        let snapshot = Snapshot::capture_selected(&parsed, &selected)
            .map_err(EditError::from)?
            .into_owned();
        item.expand_fields |= !item.authoring
            && item.draft.is_some()
            && snapshot.selection() != item.snapshot.selection();
        item.snapshot = snapshot;
        options.fields.push(field);
    }
    let pure_check = options.checks_only();
    let requested_operation = options.generates_candidate();
    if interaction == Interaction::None
        && requested_operation
        && !pure_check
        && options.set.is_empty()
        && options.add.is_empty()
        && !options.hash
        && options.hash_sources.is_empty()
        && options.prepare.is_none()
        && let Some(item) = inputs.iter().find(|item| item.draft.is_none())
    {
        return Err(EditError::at(
            Kind::InputRead,
            &item.path,
            item.snapshot.selection(),
            "no saved edits; edit values first with --set FIELD=VALUE or explicitly use --editor COMMAND".into(),
        ));
    }
    if !pure_check
        && ((!inline && options.set.is_empty() && options.add.is_empty())
            || options.prepare.is_some()
            || options.hash
            || !options.hash_sources.is_empty())
    {
        create_drafts(&mut inputs)?;
    }
    if interaction == Interaction::Editor {
        let paths = inputs
            .iter()
            .filter_map(|item| item.draft.as_deref())
            .collect::<Vec<_>>();
        crate::editor::open(
            &paths,
            options.editor.as_deref(),
            inputs[0]
                .path
                .parent()
                .expect("canonical source has a parent"),
        )
        .map_err(|error| {
            EditError::from(format!(
                "{error}\nEdits retained: {}",
                inputs[0].draft_dir.display()
            ))
        })?;
    }
    if inline || !options.set.is_empty() || !options.add.is_empty() {
        for item in &mut inputs {
            item.assign(options, inline)?;
        }
    }
    if !options.repair_missing && !matches!(options.format, Some(ReportFormat::Toml)) {
        for item in &inputs {
            if item.input_changed == Some(false) && !options.hash && options.hash_sources.is_empty()
            {
                output_cli::stderr()
                    .message(
                        HumanLevel::Info,
                        Some(&item.subject),
                        format_args!("Nothing changed"),
                    )
                    .map_err(|error| error.to_string())?;
            } else if let Some(path) = &item.draft {
                output_cli::stderr()
                    .message(
                        HumanLevel::Info,
                        Some(&item.subject),
                        format_args!("saved: {}", output_cli::human_path(path).display()),
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
    let unchanged = match item.ensure_unchanged() {
        Err(error) if !options.apply || options.hash || !options.hash_sources.is_empty() => {
            return Err(error);
        }
        result => result,
    };
    let mut document = item.document()?;
    if item.expand_fields {
        draft::complete_missing(&mut document, item.snapshot.document());
    }
    if options.repair_missing
        && options
            .upgrade
            .as_ref()
            .is_none_or(|r| !r.applies_candidate())
        && document != *item.snapshot.document()
    {
        return Err(EditError::from(
            "WORK has pending TOML edits; apply or resolve them before auto-fix",
        ));
    }
    if options.repair_missing
        && let Some(summary) = document
            .get_mut("package")
            .and_then(|value| value.get_mut("summary"))
        && let Some(fixed) = summary
            .as_str()
            .and_then(crate::check::metadata::fixed_summary)
    {
        *summary = toml::Value::String(fixed.to_owned());
    }
    if options.hash || !options.hash_sources.is_empty() {
        complete_hashes(item, &mut document, options)?;
    }
    let mut candidate = candidate::prepare(
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
    })?;
    if item.authoring {
        let path = item.draft.as_ref().expect("authoring path");
        let manifest = crate::render::manifest::parse_document(draft::read_document_path(path)?)
            .map_err(|error| EditError::from(error.to_string()))?;
        let spec =
            crate::render::run(&manifest).map_err(|error| EditError::from(error.to_string()))?;
        candidate.report = (options.check || options.apply)
            .then(|| crate::check::analyze(&spec, crate::check::Policy::Authoring, &[]));
        candidate.spec = spec;
    }
    if options.repair_missing
        && let Some(cleanup) =
            crate::spec::signature::cleanup(&candidate.spec, &options.defines, true)
                .map_err(EditError::from)?
    {
        let original = ParsedSpec::parse(item.snapshot.source());
        let old =
            crate::spec::sources::resolve(&original, &options.defines).map_err(EditError::from)?;
        for number in &cleanup.removed {
            if let Some(url) = old.sources.get(number).and_then(|s| s.url.as_ref().ok()) {
                let name = crate::spec::sources::filename(url).map_err(EditError::from)?;
                // A retained Source may intentionally share a local filename.
                let shared = old.sources.iter().any(|(n, source)| {
                    !cleanup.removed.contains(n)
                        && source
                            .url
                            .as_ref()
                            .is_ok_and(|url| crate::spec::sources::filename(url) == Ok(name))
                });
                if shared {
                    continue;
                }
                let path = item.path.parent().ok_or("SPEC has no parent")?.join(name);
                match crate::file_digest::read(&path) {
                    Ok(content) => {
                        if !candidate.removed_materials.iter().any(|(p, _)| p == &path) {
                            candidate.removed_materials.push((path, content));
                        }
                    }
                    Err(crate::file_digest::Error::Io(e))
                        if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(EditError::from(e.to_string())),
                }
            }
        }
        candidate.spec = cleanup.spec;
        candidate.source_numbers = Some(cleanup.numbers);
        candidate.changed_fields.push("sources".into());
        candidate.report = (options.check || options.apply).then(|| {
            crate::check::analyze(
                &candidate.spec,
                crate::check::Policy::Authoring,
                &options.defines,
            )
        });
    }
    if options.repair_missing
        && let Some((spec, fields)) =
            crate::spec::policy_fix::apply(&candidate.spec).map_err(EditError::from)?
    {
        candidate.spec = spec;
        candidate.changed_fields.extend(fields);
        candidate.report = (options.check || options.apply).then(|| {
            crate::check::analyze(
                &candidate.spec,
                crate::check::Policy::Authoring,
                &options.defines,
            )
        });
    }
    if let Err(error) = unchanged {
        // Retry a completed publication, never trust a cached candidate or overwrite drift.
        if static_check_error(item, &candidate).is_some()
            || !utf8_file::is_unchanged(&item.path, candidate.spec.source())
                .map_err(|error| error.to_string())?
        {
            return Err(error);
        }
        item.snapshot =
            Snapshot::capture_selected(&candidate.spec, item.snapshot.selection())?.into_owned();
        item.baseline = Some(crate::check::analyze(
            &candidate.spec,
            crate::check::Policy::Authoring,
            &[],
        ));
    }
    Ok(candidate)
}

fn complete_hashes(
    item: &mut Edit,
    document: &mut Table,
    options: &Options,
) -> Result<(), EditError> {
    if options.repair_missing
        && options
            .upgrade
            .as_ref()
            .is_none_or(|r| !r.applies_candidate())
        && document
            .get("sources")
            .and_then(toml::Value::as_table)
            .is_none_or(|sources| {
                sources.values().all(|source| {
                    source
                        .get("sha256")
                        .and_then(toml::Value::as_str)
                        .is_some_and(|hash| !hash.is_empty())
                })
            })
    {
        // No digest was requested. Unrelated URL evaluation cannot make a local repair safer.
        item.ensure_unchanged()?;
        if let Some(path) = &item.draft {
            item.save_document(path, document)?;
        }
        return Ok(());
    }
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
    let pending = if options.repair_missing {
        crate::spec::signature::remove_unused(&pending, &options.defines)
            .map_err(EditError::from)?
            .unwrap_or(pending)
    } else {
        pending
    };
    let resolved = crate::spec::sources::resolve(&pending, &options.defines).map_err(|error| {
        EditError::source_hash(
            crate::source::Error::resolution(error),
            &item.path,
            item.snapshot.selection(),
        )
    })?;
    let mut urls = if options.hash {
        crate::source::prepare_remote(&resolved)
    } else {
        crate::source::prepare_selected(&resolved, &options.hash_sources)
    }
    .map_err(|error| EditError::source_hash(error, &item.path, item.snapshot.selection()))?;
    if options.repair_missing {
        let baseline = ParsedSpec::parse(item.snapshot.source());
        let original =
            crate::spec::sources::resolve(&baseline, &options.defines).map_err(EditError::from)?;
        let upgrading = options
            .upgrade
            .as_ref()
            .is_some_and(|r| r.applies_candidate());
        // Fragments name local archives; only the HTTP request identifies remote bytes.
        let changed: std::collections::BTreeSet<_> =
            urls.iter()
                .filter_map(|(&number, remote)| {
                    let same = original.sources.get(&number).is_some_and(|old| {
                        old.url.as_ref().is_ok_and(|url| remote.same_request(url))
                    });
                    (!same).then_some(number)
                })
                .collect();
        if upgrading && changed.is_empty() {
            return Err(EditError::from(
                "upgrade did not change any remote Source request URL; update the source mapping before applying",
            ));
        }
        urls.retain(|number, _| {
            matches!(resolved.sources[number].digest, Ok(None))
                || (upgrading && changed.contains(number))
        });
    }
    for number in urls.keys() {
        let field = format!("sources.{number}.sha256");
        if options
            .set
            .iter()
            .chain(&options.add)
            .any(|(key, _)| key == &field)
        {
            return Err(EditError::at(
                Kind::SourceHashFailed,
                &item.path,
                item.snapshot.selection(),
                format!("{field}: choose either --set/--add or --hash, not both"),
            ));
        }
        if crate::spec::document::table::lookup(document, &field).is_none() {
            return Err(EditError::at(
                Kind::SourceHashFailed,
                &item.path,
                item.snapshot.selection(),
                format!("{field}: not selected in this draft. Prepare a new draft with this field"),
            ));
        }
    }
    item.source_hashes = Some(crate::source::SourceHashes {
        input_sha256: utf8_file::sha256(pending.source()),
        defines: options.defines.clone(),
        sources: Default::default(),
    });
    crate::source::download_prepared(
        urls,
        item.development
            .as_ref()
            .map(workspace::Development::sources)
            .as_deref(),
        &mut item.source_hashes.as_mut().expect("hash operation").sources,
    )
    .map_err(|error| EditError::source_hash(error, &item.path, item.snapshot.selection()))?;
    item.ensure_unchanged()?;
    draft::complete_digests(
        document,
        &item.source_hashes.as_ref().expect("hash operation").sources,
    )?;
    if let Some(path) = &item.draft {
        item.save_document(path, document)?;
    }
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
            if !item.authoring {
                draft::protect_output(output, item.draft.as_deref())?;
            }
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
                output_cli::select_edit_action(path, options.format.unwrap_or(ReportFormat::Human))
            }
        },
    )
    .map_err(EditError::publication)?;
    for item in inputs {
        if outcomes.iter().any(|outcome|matches!(outcome,file_output::EditOutcome::Written(path)|file_output::EditOutcome::Unchanged(path) if path==&item.path))
            && let Ok(candidate) = item.result()
        {
            for (path, expected) in &candidate.removed_materials {
            let deletion = (|| -> io::Result<()> {
                let actual=crate::file_digest::read(path).map_err(io::Error::other)?;
                if &actual!=expected {return Err(io::Error::other("signature file changed; retained"));}
                fs::remove_file(path)
            })();
            if let Err(error)=deletion {
                return Err(EditError::recovery(file_output::OutputError::Partial {
                    written: outcomes.iter().filter_map(|o|match o{file_output::EditOutcome::Written(p)=>Some(p.clone()),_=>None}).collect(),
                    source:Box::new(file_output::OutputError::Write{path:path.clone(),source:error}),
                }, "SPEC was written; signature file removal failed".into()));
            }
        }
    }
    }
    rebase_drafts(inputs, &outcomes)?;
    if !matches!(options.format, Some(ReportFormat::Toml)) {
        for outcome in &outcomes {
            output_cli::write_outcome(&mut output_cli::stderr(), outcome)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(outcomes)
}

fn rebase_drafts(inputs: &[Edit], outcomes: &[file_output::EditOutcome]) -> Result<(), EditError> {
    // Only publication to the bound source advances its recovery baseline.
    // A copy output leaves the original binding and its pending edit intact.
    for item in inputs {
        if outcomes.iter().any(|outcome| matches!(outcome,
            file_output::EditOutcome::Written(path) | file_output::EditOutcome::Unchanged(path) if path == &item.path))
            && !item.authoring
            && let Some(path) = &item.draft
        {
            let candidate = item.result().expect("published candidate");
            let selection: Vec<_> = item.snapshot.selection().iter().filter_map(|field| {
                let Some(mapping) = &candidate.source_numbers else { return Some(field.clone()) };
                let Some(rest) = field.strip_prefix("sources.") else { return Some(field.clone()) };
                let (number, suffix) = rest.split_once('.').map_or((rest, ""), |(n,s)| (n,s));
                let number: u32 = number.parse().ok()?;
                mapping.get(&number).map(|new| if suffix.is_empty() { format!("sources.{new}") } else { format!("sources.{new}.{suffix}") })
            }).collect();
            let document = Snapshot::capture_selected(&candidate.spec, &selection)
                .map_err(EditError::from)?;
            draft::update(path, &selection, document.document(), Some(candidate.spec.source()))
                .map_err(|error| {
                    let message = format!("SPEC publication completed, but draft recovery state could not advance: {error}; retry with edit --from {} --apply after fixing the state storage", shell_words::quote(&item.draft_dir.to_string_lossy()));
                    EditError::recovery(file_output::OutputError::Partial {
                    written: outcomes.iter().filter_map(|outcome| match outcome {
                        file_output::EditOutcome::Written(path) => Some(path.clone()), _ => None,
                    }).collect(),
                    source: Box::new(file_output::OutputError::Write { path: path.clone(), source: io::Error::other(error) }),
                }, message)
                })?;
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
    fn success_for(&self, options: &Options) -> bool {
        self.is_success() && (!options.repair_missing || self.static_valid() == Some(true))
    }

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

#[cfg(test)]
mod tests {
    #[test]
    fn version_change_removes_the_original_signature_material() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ed.spec");
        let source = include_str!("../tests/fixtures/ed.spec").replace(
            "BuildSystem:",
            "#!RemoteAsset\nSource1: https://example.org/ed-%{version}.sign\nBuildSystem:",
        );
        std::fs::write(&path, &source).unwrap();
        let signature = dir.path().join("ed-1.22.5.sign");
        std::fs::write(&signature, b"signature bytes").unwrap();
        let mut options = super::Options {
            specs: vec![path.clone()],
            set: vec![("package.version".into(), "1.22.6".into())],
            repair_missing: true,
            apply: true,
            check: true,
            format: Some(crate::output_cli::ReportFormat::Toml),
            upgrade: Some(Box::new(crate::check::upgrade::Report {
                project: Some("ed".into()),
                current: Some("1.22.5".into()),
                candidate: Some("1.22.6".into()),
                status: "upgrade-available",
                error: None,
                input_sha256: crate::utf8_file::sha256(&source),
            })),
            ..Default::default()
        };
        let result = super::execute(&mut options).unwrap();
        assert!(
            result.success_for(&options),
            "publication={:?}; candidates={:?}",
            result.publication.as_ref().err().map(ToString::to_string),
            result
                .inputs
                .iter()
                .map(|item| item.candidate.as_ref().map(|r| match r {
                    Err(e) => e.to_string(),
                    Ok(c) => format!(
                        "checked={:?}",
                        c.report
                            .as_ref()
                            .map(crate::check_report::CheckReport::is_success)
                    ),
                }))
                .collect::<Vec<_>>()
        );
        assert!(!signature.exists());
        let edited = std::fs::read_to_string(path).unwrap();
        assert!(edited.contains("Version:        1.22.6"));
        assert!(!edited.contains("Source1:"));
    }
}
