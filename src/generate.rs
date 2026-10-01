// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Completion and source-preserving generation inside one bound WORK.

use serde::Serialize;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    path::{Path, PathBuf},
};
use toml::{Table, Value};

use crate::{
    check_report::CheckReport,
    file_output,
    output_cli::{self, ReportFormat},
    render, source,
    spec::{ParsedSpec, document::Snapshot},
    stage, utf8_file, workspace,
};

#[derive(clap::Args)]
#[command(
    after_help = "Reads the current input saved in WORK: authoring PKG.toml or source-bound stage.\nUse --input authoring|edit to select explicitly; preview/check does not change the selection.\nDefault: writes a derived PKG.resolved.toml and caches the candidate SPEC; checkout is unchanged.\n--spec auto publishes checkout/SPECS/PKG/PKG.spec; --spec . writes beside the resolved TOML.\n--spec PATH names the complete output SPEC file. Every SPEC publication is checked first.\nMissing remote Source digests are downloaded automatically; --hash recalculates all remote digests.\n--offline disables downloads. --check, --stdout and --diff do not publish files."
)]
pub(crate) struct Options {
    /// Existing development area whose saved package binding selects the input.
    #[arg(value_name = "WORK")]
    work: String,
    /// Verify the package binding; an existing WORK cannot be rebound.
    #[arg(long, value_name = "PKG")]
    pkgname: Option<String>,
    /// Select authoring TOML or the saved edit stage; persists after successful generation.
    #[arg(long, value_enum)]
    input: Option<workspace::GenerationInput>,
    /// Publish SPEC to auto, beside the resolved TOML (.), or an explicit file.
    #[arg(long, value_name = "auto|.|PATH")]
    spec: Option<PathBuf>,
    /// Recalculate every remote Source SHA-256, including existing digests.
    #[arg(long, conflicts_with = "offline")]
    hash: bool,
    /// Disable downloads for missing Source SHA-256 digests.
    #[arg(long)]
    offline: bool,
    /// Check without publishing resolved TOML, cached SPEC or checkout files.
    #[arg(long, conflicts_with_all = ["stdout", "diff", "force", "skip_existing"])]
    check: bool,
    /// Select the generation check report format.
    #[arg(long, value_enum, requires = "check", conflicts_with_all = ["stdout", "diff", "force", "skip_existing"])]
    format: Option<ReportFormat>,
    #[command(flatten, next_help_heading = "Output options")]
    output: output_cli::OutputActionOptions,
}

struct Input {
    path: PathBuf,
    original_document: String,
    document: Table,
    bound: Option<stage::Draft>,
}

impl Input {
    fn unchanged(&self) -> Result<(), GenerateError> {
        for (path, original) in std::iter::once((&self.path, &self.original_document)).chain(
            self.bound
                .as_ref()
                .map(|draft| (&draft.source, &draft.original)),
        ) {
            if !utf8_file::is_unchanged(path, original).map_err(|source| GenerateError::Read {
                path: path.clone(),
                source,
            })? {
                return Err(GenerateError::InputChanged(path.clone()));
            }
        }
        Ok(())
    }
}

struct Generated {
    spec: ParsedSpec<'static>,
    report: Option<CheckReport>,
    baseline: Option<CheckReport>,
    // Owned once: the exact validated values used by the renderer and the resolved snapshot.
    manifest: Option<render::manifest::Manifest>,
}

impl Generated {
    fn admissible(&self) -> bool {
        self.report.as_ref().is_none_or(|report| {
            self.baseline.as_ref().map_or_else(
                || report.is_success(),
                |baseline| report.allows_edit(baseline),
            )
        })
    }
}

#[derive(Serialize)]
struct GenerationReport<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    scope: &'static str,
    valid: Option<bool>,
    admissible: Option<bool>,
    success: bool,
    baseline_report: Option<crate::check_report::Report<'a>>,
    input: crate::report::Input<'a>,
    work: &'a str,
    profile: crate::profile::Identity,
    build_contract: Option<crate::profile::Identity>,
    report: Option<crate::check_report::Report<'a>>,
    source_hashes: Option<crate::report::Numbered<'a, source::Download>>,
    source_hash_failures: crate::report::Numbered<'a, source::Error>,
    authoring_warnings: &'a [String],
    error: Option<crate::report::Failure>,
}

/// WORK explicitly selects the authority; derived files never become inputs.
pub(crate) fn run(options: &Options) -> Result<bool, GenerateError> {
    let mut manifest_path = None;
    let mut baseline_path = None;
    let mut manifest_digest = None;
    let mut hashes = (!options.offline).then(BTreeMap::new);
    let mut failures = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut candidate = None;
    let mut target = None;
    let mut written = Vec::new();
    let result = (|| {
        let workspace = workspace::discover().map_err(GenerateError::Workspace)?;
        let mut development = workspace
            .existing_development(&options.work)
            .map_err(GenerateError::Workspace)?;
        if options
            .pkgname
            .as_deref()
            .is_some_and(|name| name != development.package())
        {
            return Err(GenerateError::Invalid(format!(
                "{} is bound to package {}; --pkgname cannot change an existing binding",
                options.work,
                development.package(),
            )));
        }
        let selected_input = options.input.unwrap_or(development.generation_input());
        let selected_path = match selected_input {
            workspace::GenerationInput::Authoring => development.manifest(),
            workspace::GenerationInput::Edit => development
                .directory()
                .join("stage")
                .join(format!("{}.toml", development.package())),
        };
        manifest_path = Some(selected_path);
        let mut input = load(
            &development,
            selected_input,
            &mut manifest_path,
            &mut manifest_digest,
        )?;
        baseline_path = input.bound.as_ref().map(|draft| draft.source.clone());
        let resolved_path = input
            .path
            .with_file_name(format!("{}.resolved.toml", development.package()));
        let selected_target = spec_target(options, &development, &resolved_path);
        target = Some(selected_target.clone());
        if !options.output.stdout && !options.output.diff {
            protect_target(&input, &selected_target, &resolved_path, &development)?;
        }
        let generated = if input.bound.is_some() {
            from_stage(
                &mut input,
                options,
                &mut hashes,
                &mut failures,
                &mut warnings,
            )?
        } else {
            from_authoring(
                &mut input,
                development.package(),
                options,
                &mut hashes,
                &mut failures,
                &mut warnings,
            )?
        };
        let valid = generated.admissible();
        candidate = Some(generated);
        if options.check {
            return Ok(valid);
        }
        if !valid {
            return Err(GenerateError::CheckFailed);
        }
        input.unchanged()?;
        let generated = candidate.as_ref().expect("generated candidate");
        if options.output.diff
            && options.spec.is_none()
            && let Some(bound) = &input.bound
        {
            write!(
                io::stdout().lock(),
                "{}",
                stage::diff(&bound.source, &bound.original, generated.spec.source())
                    .map_err(GenerateError::Invalid)?
            )
            .map_err(GenerateError::Stdout)?;
            return Ok(true);
        }
        if options.output.stdout || options.output.diff {
            options
                .output
                .emit(&selected_target, generated.spec.source())
                .map_err(GenerateError::Output)?;
            return Ok(true);
        }
        // Explicit publication is the gate: no derived artifacts appear when
        // the check or destination fails. Only auto needs a checkout.
        if let Some(spec) = &options.spec {
            if spec == Path::new("auto") {
                // Retain the same WORK lock from input selection through publication.
                workspace
                    .materialize(&mut development)
                    .map_err(GenerateError::Workspace)?;
            }
            let source_path = input
                .bound
                .as_ref()
                .map_or(&input.path, |draft| &draft.source);
            let original = input
                .bound
                .as_ref()
                .map_or(input.original_document.as_str(), |draft| {
                    draft.original.as_str()
                });
            let outcomes = options
                .output
                .emit_from(
                    &selected_target,
                    generated.spec.source(),
                    source_path,
                    original,
                )
                .map_err(GenerateError::Output)?;
            written.extend(outcomes.into_iter().filter_map(|outcome| match outcome {
                file_output::EditOutcome::Written(path) => Some(path),
                _ => None,
            }));
            if spec == Path::new("auto")
                && let Some(bound) = &input.bound
            {
                // A skipped conflict is not publication; rebase only observed bytes.
                if utf8_file::read(&selected_target).map_err(GenerateError::Input)?
                    == generated.spec.source()
                {
                    let fields = &bound.fields;
                    let baseline = Snapshot::capture_selected(&generated.spec, fields)
                        .map_err(GenerateError::Invalid)?;
                    // Rebase the exact published declarations, not the derived
                    // completion table that intentionally omits unobserved hashes.
                    stage::update(
                        &bound.path,
                        fields,
                        baseline.document(),
                        Some(generated.spec.source()),
                    )
                    .map_err(GenerateError::Invalid)?;
                }
            }
        }
        if options.spec.is_none() {
            input.unchanged()?;
        }
        let document = resolved_document(&input, generated, hashes.as_ref(), &failures)?;
        file_output::write_artifact(&resolved_path, document.as_bytes()).map_err(|source| {
            GenerateError::ArtifactWrite {
                path: resolved_path.clone(),
                source,
            }
        })?;
        let dir = resolved_path.parent().expect("resolved TOML has a parent");
        let cache_dir = if input.bound.is_some() {
            dir.to_path_buf()
        } else {
            dir.join("stage")
        };
        stage::cache(&cache_dir, development.package(), generated.spec.source())
            .map_err(GenerateError::Invalid)?;
        if let Some(bound) = &input.bound {
            let diff = stage::diff(&bound.source, &bound.original, generated.spec.source())
                .map_err(GenerateError::Invalid)?;
            stage::save_diff(&cache_dir, development.package(), &diff)
                .map_err(GenerateError::Invalid)?;
        }
        if options.input.is_some() {
            development
                .select_input(selected_input)
                .map_err(GenerateError::Workspace)?;
        }
        Ok(true)
    })()
    .map_err(|error: GenerateError| error.with_written(&written));
    if options.check && matches!(options.format, Some(ReportFormat::Toml)) {
        let success = result.as_ref().is_ok_and(|admissible| *admissible);
        let report = GenerationReport {
            format_version: 4,
            tool: crate::tool::identity(),
            scope: if candidate
                .as_ref()
                .is_some_and(|candidate| candidate.manifest.is_none())
            {
                "selected-generation-static"
            } else {
                "manifest-generation-static"
            },
            valid: candidate
                .as_ref()
                .and_then(|candidate| candidate.report.as_ref())
                .map(CheckReport::is_success),
            admissible: candidate.as_ref().map(Generated::admissible),
            success,
            baseline_report: candidate
                .as_ref()
                .and_then(|candidate| candidate.baseline.as_ref())
                .map(|report| {
                    report.structured(baseline_path.as_deref().expect("baseline source"))
                }),
            input: crate::report::Input {
                display_path: manifest_path.as_ref().map_or_else(
                    || options.work.as_str().into(),
                    |path| path.to_string_lossy(),
                ),
                sha256: manifest_digest.as_deref(),
                revision: None,
            },
            work: &options.work,
            profile: crate::profile::identity(),
            build_contract: candidate
                .as_ref()
                .and_then(|candidate| candidate.manifest.as_ref())
                .and_then(|manifest| manifest.build.system.as_deref())
                .and_then(crate::profile::buildsystems::contract_identity),
            report: candidate
                .as_ref()
                .and_then(|candidate| candidate.report.as_ref())
                .map(|report| report.structured(target.as_deref().expect("candidate target"))),
            source_hashes: hashes.as_ref().map(crate::report::Numbered),
            source_hash_failures: crate::report::Numbered(&failures),
            authoring_warnings: &warnings,
            error: result
                .as_ref()
                .err()
                .map(|error| crate::report::failure(error.code(), error)),
        };
        crate::report::write(&mut io::stdout().lock(), &report).map_err(GenerateError::Report)?;
        return Ok(success);
    }
    for (number, error) in &failures {
        output_cli::human(
            &mut io::stderr().lock(),
            output_cli::HumanLevel::Warn,
            Some(Path::new(&options.work)),
            format_args!("sources.{number}.sha256: not calculated: {error}; left missing"),
        )
        .map_err(|error| GenerateError::Stderr(error).with_written(&written))?;
    }
    for warning in &warnings {
        output_cli::human(
            &mut io::stderr().lock(),
            output_cli::HumanLevel::Warn,
            Some(Path::new(&options.work)),
            format_args!("{warning}"),
        )
        .map_err(|error| GenerateError::Stderr(error).with_written(&written))?;
    }
    if let Some(report) = candidate
        .as_ref()
        .and_then(|candidate| candidate.report.as_ref())
    {
        let subject = PathBuf::from(format!("{} (candidate)", options.work));
        report
            .write_human(&subject, &mut io::stderr().lock())
            .map_err(|error| GenerateError::Stderr(error).with_written(&written))?;
    }
    result
}

fn load(
    development: &workspace::Development,
    selected: workspace::GenerationInput,
    manifest_path: &mut Option<PathBuf>,
    manifest_digest: &mut Option<String>,
) -> Result<Input, GenerateError> {
    let bound = if matches!(selected, workspace::GenerationInput::Edit) {
        let drafts =
            stage::load(&development.directory().join("stage")).map_err(GenerateError::Invalid)?;
        let [draft]: [stage::Draft; 1] = drafts.try_into().map_err(|_| {
            GenerateError::Invalid("gen WORK requires one saved edit source".into())
        })?;
        let expected = development
            .package_directory()
            .join(format!("{}.spec", development.package()));
        if draft.source != expected {
            return Err(GenerateError::Invalid(format!(
                "indexed stage source {} is not WORK's bound checkout SPEC {}",
                draft.source.display(),
                expected.display()
            )));
        }
        Some(draft)
    } else {
        None
    };
    let path = bound
        .as_ref()
        .map_or_else(|| development.manifest(), |draft| draft.path.clone());
    *manifest_path = Some(path.clone());
    let original_document = if bound.is_some() {
        stage::read_text(&path).map_err(GenerateError::Invalid)?
    } else {
        utf8_file::read(&path).map_err(GenerateError::Input)?
    };
    *manifest_digest = Some(utf8_file::sha256(&original_document));
    let document = toml::from_str(&original_document).map_err(|source| {
        if bound.is_some() {
            GenerateError::Invalid(format!("{}: {source}", path.display()))
        } else {
            GenerateError::Render {
                path: path.clone(),
                source: render::RenderError::Toml(source),
            }
        }
    })?;
    Ok(Input {
        path,
        original_document,
        document,
        bound,
    })
}

fn from_authoring(
    input: &mut Input,
    package: &str,
    options: &Options,
    hashes: &mut Option<BTreeMap<u32, source::Download>>,
    failures: &mut BTreeMap<u32, source::Error>,
    warnings: &mut Vec<String>,
) -> Result<Generated, GenerateError> {
    if let Some(fields) = input
        .document
        .entry("package")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
    {
        fields
            .entry("name")
            .or_insert_with(|| Value::String(package.into()));
    }
    let error = |source| GenerateError::Render {
        path: input.path.clone(),
        source,
    };
    let mut manifest =
        render::manifest::parse_document(std::mem::take(&mut input.document)).map_err(error)?;
    if manifest.package.name != package {
        return Err(GenerateError::ManifestNotForPackage {
            requested: package.into(),
            path: input.path.clone(),
        });
    }
    if matches!(manifest.package.vcs, render::manifest::Vcs::Unknown) {
        warnings.push("package.vcs: repository status is unconfirmed; no repository or absence is inferred. Confirm it before publishing.".into());
    }
    let mut spec = render::run(&manifest).map_err(error)?;
    let mut report = crate::check::analyze(&spec, crate::check::Policy::Authoring, &[]);
    if let Some(downloads) = hashes.as_mut().filter(|_| report.is_success()) {
        let mut attempted = false;
        for (number, material) in &mut manifest.sources {
            let render::manifest::Source::Remote { url, sha256 } = material else {
                continue;
            };
            if sha256.is_some() && !options.hash {
                continue;
            }
            attempted = true;
            let downloaded = render::manifest::resolve_source(
                url,
                &manifest.package.name,
                &manifest.package.version,
                &manifest.package.url,
            )
            .map_err(source::Error::resolution)
            .and_then(|url| source::RemoteSource::parse(&url)?.download());
            match downloaded {
                Ok(download) => {
                    *sha256 = Some(download.sha256.clone());
                    downloads.insert(*number, download);
                }
                Err(error) => {
                    failures.insert(*number, error.at(*number));
                }
            }
        }
        if attempted {
            input.unchanged()?;
        }
        if options.hash && !failures.is_empty() {
            return Err(GenerateError::HashFailed);
        }
        if !downloads.is_empty() {
            spec = render::run(&manifest).map_err(error)?;
            report = crate::check::analyze(&spec, crate::check::Policy::Authoring, &[]);
        }
    }
    Ok(Generated {
        spec,
        report: Some(report),
        baseline: None,
        manifest: Some(manifest),
    })
}

fn from_stage(
    input: &mut Input,
    options: &Options,
    hashes: &mut Option<BTreeMap<u32, source::Download>>,
    failures: &mut BTreeMap<u32, source::Error>,
    warnings: &mut Vec<String>,
) -> Result<Generated, GenerateError> {
    let bound = input.bound.as_ref().expect("source-bound stage");
    let explicit_digests = input
        .document
        .get("sources")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|sources| sources.iter())
        .filter_map(|(key, value)| {
            value
                .as_table()
                .filter(|source| source.contains_key("sha256"))
                .and_then(|_| key.parse::<u32>().ok())
        })
        .collect::<BTreeSet<_>>();
    let parsed = ParsedSpec::parse(&bound.original);
    let mut fields = bound.fields.clone();
    let mut snapshot =
        Snapshot::capture_selected(&parsed, &fields).map_err(GenerateError::Invalid)?;
    stage::complete_missing(&mut input.document, snapshot.document());
    // Resolve the edited candidate, not the old source's URL or version.
    let pending = snapshot
        .render_before_hashing(&input.document, &[])
        .map_err(GenerateError::Invalid)?;
    let resolved = crate::spec::sources::resolve(&pending, &[]).map_err(GenerateError::Invalid)?;
    let mut numbers = Vec::new();
    let original_sources =
        crate::spec::sources::resolve(&parsed, &[]).map_err(GenerateError::Invalid)?;
    let changed_urls = resolved
        .sources
        .iter()
        .filter_map(|(number, material)| {
            let before = original_sources.sources.get(number)?;
            match (&before.url, &material.url) {
                (Ok(before), Ok(after)) if before != after => Some(*number),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    for number in &changed_urls {
        warnings.push(format!("sources.{number}: effective URL changed; any retained digest is a declaration, not verification of the new URL; use --hash to recalculate"));
    }
    let complete_sources = resolved.incomplete.is_none();
    if !options.offline && !complete_sources {
        let reason = resolved
            .incomplete
            .as_deref()
            .expect("incomplete source resolution");
        if options.hash {
            return Err(GenerateError::Invalid(format!(
                "cannot recalculate Source digests: {reason}"
            )));
        }
        warnings.push(format!("sources: automatic digest completion skipped: {reason}; unresolved facts were not inferred"));
    }
    if !options.offline && complete_sources {
        for (number, material) in &resolved.sources {
            let Ok(url) = &material.url else {
                continue;
            };
            if !source::is_remote_url(url) {
                continue;
            }
            let supplied = crate::spec::document::table::lookup(
                &input.document,
                &format!("sources.{number}.sha256"),
            )
            .and_then(Value::as_str)
            .is_some_and(|digest| !digest.is_empty());
            let missing_changed_digest =
                changed_urls.contains(number) && !explicit_digests.contains(number);
            if !options.hash
                && !missing_changed_digest
                && (supplied || material.digest.as_ref().is_ok_and(Option::is_some))
            {
                continue;
            }
            numbers.push(*number);
            let field = format!("sources.{number}.sha256");
            if !fields.is_empty()
                && !fields.iter().any(|selected| {
                    selected == &field || field.starts_with(&format!("{selected}."))
                })
            {
                fields.push(field);
            }
        }
    }
    if fields != bound.fields {
        snapshot = Snapshot::capture_selected(&parsed, &fields).map_err(GenerateError::Invalid)?;
        stage::complete_missing(&mut input.document, snapshot.document());
    }
    if !numbers.is_empty() {
        for number in &numbers {
            // Selection already proved these URLs against the edited candidate;
            // adding digest fields does not change URL resolution.
            let url = resolved.sources[number]
                .url
                .as_ref()
                .expect("selected resolved URL");
            match source::RemoteSource::parse(url).and_then(source::RemoteSource::download) {
                Ok(download) => {
                    let field = format!("sources.{number}.sha256");
                    *crate::spec::document::table::lookup_mut(&mut input.document, &field)
                        .ok_or_else(|| {
                            GenerateError::Invalid(format!("{field}: digest mapping unavailable"))
                        })? = Value::String(download.sha256.clone());
                    hashes
                        .as_mut()
                        .expect("downloads enabled")
                        .insert(*number, download);
                }
                Err(error) => {
                    failures.insert(*number, error.at(*number));
                }
            }
        }
        input.unchanged()?;
        if options.hash && !failures.is_empty() {
            return Err(GenerateError::HashFailed);
        }
    }
    let generated = crate::spec::candidate::prepare(
        &snapshot,
        &input.document,
        &[],
        options.check || options.spec.is_some(),
    )
    .map_err(GenerateError::Invalid)?;
    // A missing input digest must not turn an old URL's declaration into a
    // resolved fact for the edited URL. Rendering preserves the old SPEC
    // marker, but the derived TOML leaves this unobserved value absent.
    for number in &changed_urls {
        if !explicit_digests.contains(number)
            && hashes
                .as_ref()
                .is_none_or(|hashes| !hashes.contains_key(number))
            && let Some(source) = input
                .document
                .get_mut("sources")
                .and_then(Value::as_table_mut)
                .and_then(|sources| sources.get_mut(&number.to_string()))
                .and_then(Value::as_table_mut)
        {
            source.remove("sha256");
        }
    }
    let baseline = (options.check || options.spec.is_some())
        .then(|| crate::check::analyze(&parsed, crate::check::Policy::Authoring, &[]));
    input.bound.as_mut().expect("source-bound stage").fields = fields;
    Ok(Generated {
        spec: generated.spec,
        report: generated.report,
        baseline,
        manifest: None,
    })
}

/// A read-only view of the same prepared facts used to render the candidate.
/// Source-bound edits retain their baseline; they are not full authoring manifests.
fn resolved_document(
    input: &Input,
    generated: &Generated,
    hashes: Option<&BTreeMap<u32, source::Download>>,
    failures: &BTreeMap<u32, source::Error>,
) -> Result<String, GenerateError> {
    #[derive(Serialize)]
    struct Edit<'a> {
        source: Cow<'a, str>,
        original_sha256: String,
        fields: &'a [String],
        values: &'a Table,
    }
    #[derive(Serialize)]
    struct Resolved<'a> {
        format_version: u32,
        role: &'static str,
        input: crate::report::Input<'a>,
        spec_sha256: String,
        profile_identity: crate::profile::Identity,
        profile: Option<&'a crate::profile::Profile>,
        manifest: Option<&'a render::manifest::Manifest>,
        edit: Option<Edit<'a>>,
        downloads: Option<crate::report::Numbered<'a, source::Download>>,
        download_failures: crate::report::Numbered<'a, source::Error>,
    }
    let sha256 = utf8_file::sha256(&input.original_document);
    let resolved = Resolved {
        format_version: 2,
        role: if input.bound.is_some() {
            "resolved-edit"
        } else {
            "resolved-authoring"
        },
        input: crate::report::Input {
            display_path: input.path.to_string_lossy(),
            sha256: Some(&sha256),
            revision: None,
        },
        spec_sha256: utf8_file::sha256(generated.spec.source()),
        profile_identity: crate::profile::identity(),
        profile: generated.manifest.as_ref().map(|_| crate::profile::load()),
        manifest: generated.manifest.as_ref(),
        edit: input.bound.as_ref().map(|bound| Edit {
            source: bound.source.to_string_lossy(),
            original_sha256: utf8_file::sha256(&bound.original),
            fields: &bound.fields,
            values: &input.document,
        }),
        downloads: hashes.map(crate::report::Numbered),
        download_failures: crate::report::Numbered(failures),
    };
    toml::to_string_pretty(&resolved).map_err(|error| GenerateError::Invalid(error.to_string()))
}

fn spec_target(
    options: &Options,
    development: &workspace::Development,
    resolved_path: &Path,
) -> PathBuf {
    match options.spec.as_deref() {
        Some(path) if path == Path::new("auto") => development
            .package_directory()
            .join(format!("{}.spec", development.package())),
        Some(path) if path == Path::new(".") => {
            resolved_path.with_file_name(format!("{}.spec", development.package()))
        }
        Some(path) => path.to_owned(),
        None => development
            .directory()
            .join("stage")
            .join(format!("{}.candidate.spec", development.package())),
    }
}

fn protect_target(
    input: &Input,
    target: &Path,
    resolved_path: &Path,
    development: &workspace::Development,
) -> Result<(), GenerateError> {
    development
        .protect_output(target)
        .map_err(|_| GenerateError::InputIsTarget(target.to_owned()))?;
    let work = development.directory();
    let target_identity = file_output::output_path(target).ok();
    for protected in [&input.path, resolved_path] {
        if target_identity.as_deref() == Some(protected) || file_output::aliases(target, protected)
        {
            return Err(GenerateError::InputIsTarget(target.to_owned()));
        }
    }
    if target_identity
        .as_ref()
        .is_some_and(|target| target.starts_with(work.join("stage/.state")))
    {
        return Err(GenerateError::Invalid(format!(
            "SPEC output {} conflicts with WORK input or completion state",
            target.display()
        )));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GenerateError {
    #[error("{0}")]
    Workspace(#[source] io::Error),
    #[error("input {} changed during generation; no derived files published; rerun against the current input", .0.display())]
    InputChanged(PathBuf),
    #[error("failed to write generation report: {0}")]
    Report(#[source] io::Error),
    #[error("failed to write output to stdout: {0}")]
    Stdout(#[source] io::Error),
    #[error("{0}")]
    Input(#[source] utf8_file::Utf8FileError),
    #[error("failed to generate SPEC from {}: {source}", .path.display())]
    Render {
        path: PathBuf,
        #[source]
        source: render::RenderError,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Output(#[source] file_output::OutputError),
    #[error("failed to read {}: {source}", .path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to write derived artifact {}: {source}", .path.display())]
    ArtifactWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "{source}\nSPEC files already written: {written:?}\nPublication was not rolled back; inspect these paths before retrying."
    )]
    AfterPublication {
        #[source]
        source: Box<GenerateError>,
        written: Vec<PathBuf>,
    },
    #[error("input path conflicts with generated target {}", .0.display())]
    InputIsTarget(PathBuf),
    #[error("manifest at {} is not for requested package {requested:?}", .path.display())]
    ManifestNotForPackage { requested: String, path: PathBuf },
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
    #[error("generated SPEC failed the selected static checks")]
    CheckFailed,
    #[error("--hash failed to recalculate every remote Source; no derived files published")]
    HashFailed,
}

impl GenerateError {
    fn with_written(self, written: &[PathBuf]) -> Self {
        if written.is_empty() {
            self
        } else {
            Self::AfterPublication {
                source: Box::new(self),
                written: written.to_vec(),
            }
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Input(_) | Self::Read { .. } | Self::Workspace(_) => "input-read",
            Self::InputChanged(_) => "source-changed",
            Self::Render { .. } | Self::Invalid(_) => "generation-failed",
            Self::ManifestNotForPackage { .. } => "package-mismatch",
            Self::InputIsTarget(_) => "input-is-target",
            Self::Output(_) => "publication-failed",
            Self::AfterPublication { source, .. } => source.code(),
            Self::ArtifactWrite { .. } | Self::Report(_) | Self::Stdout(_) | Self::Stderr(_) => {
                "output-write"
            }
            Self::CheckFailed => "static-check-failed",
            Self::HashFailed => "source-hash-failed",
        }
    }
}
