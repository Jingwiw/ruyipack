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
    draft, file_output,
    output_cli::{self, ReportFormat},
    render, source,
    spec::{ParsedSpec, document::Snapshot},
    utf8_file, workspace,
};

#[derive(clap::Args)]
#[command(
    after_help = "Read the WORK TOML. Imported SPEC text remains bound to its original bytes.
Save resolved TOML and a candidate SPEC; keep the recipe unchanged.
--diff shows changes. --apply checks and writes the recipe SPEC.
--output FILE selects another destination. --output . writes beside the input TOML.
Download missing Source digests; --hash refreshes existing digests too.
--offline prevents downloads. --check and --stdout save no SPEC or TOML; downloads can be cached."
)]
pub(crate) struct Options {
    /// Existing development area whose saved package binding selects the input.
    #[arg(value_name = "WORK")]
    work: String,
    /// Publish the checked candidate to the bound recipe directory.
    #[arg(long, conflicts_with_all = ["destination", "check", "stdout"])]
    apply: bool,
    /// Export the checked SPEC to this file; . places it beside the resolved TOML.
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    destination: Option<PathBuf>,
    /// Recalculate every remote Source SHA-256, including existing digests.
    #[arg(long, conflicts_with = "offline")]
    hash: bool,
    /// Disable downloads for missing Source SHA-256 digests.
    #[arg(long)]
    offline: bool,
    /// Check without publishing resolved TOML, cached SPEC or recipe files.
    #[arg(long, conflicts_with_all = ["stdout", "force", "skip_existing"])]
    check: bool,
    /// Select the operation report format, independently of checking or publication.
    #[arg(long, value_enum, conflicts_with = "stdout")]
    format: Option<ReportFormat>,
    /// Print the candidate SPEC without publishing it.
    #[arg(long, conflicts_with_all = ["diff", "force", "skip_existing"])]
    stdout: bool,
    /// Show candidate differences; publish only when combined with --apply.
    #[arg(long)]
    diff: bool,
    #[command(flatten, next_help_heading = "Output options")]
    output: output_cli::ConflictOptions,
}

impl Options {
    fn publishes(&self) -> bool {
        self.apply || self.destination.is_some()
    }
}

struct Input {
    path: PathBuf,
    original_document: String,
    document: Table,
    bound: Option<draft::Draft>,
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
    fn publish_spec(
        &self,
        options: &Options,
        workspace: &workspace::Workspace,
        development: &mut workspace::Development,
        target: &Path,
        generated: &Generated,
        written: &mut Vec<PathBuf>,
    ) -> Result<(), GenerateError> {
        // Explicit publication is the gate: no derived artifacts appear when
        // the check or destination fails. Only --apply needs local recipe files.
        if options.publishes() {
            if options.apply {
                // Retain the same WORK lock from input selection through publication.
                workspace
                    .materialize(development)
                    .map_err(GenerateError::Workspace)?;
            }
            let source_path = self
                .bound
                .as_ref()
                .map_or(&self.path, |draft| &draft.source);
            let original = self
                .bound
                .as_ref()
                .map_or(self.original_document.as_str(), |draft| {
                    draft.original.as_str()
                });
            let outcomes = options
                .output
                .publish_from(
                    target,
                    generated.spec.source(),
                    source_path,
                    original,
                    options.format.unwrap_or(ReportFormat::Human),
                )
                .map_err(GenerateError::Output)?;
            written.extend(outcomes.into_iter().filter_map(|outcome| match outcome {
                file_output::EditOutcome::Written(path) => Some(path),
                _ => None,
            }));
            if options.apply
                && let Some(bound) = &self.bound
            {
                // A skipped conflict is not publication; rebase only observed bytes.
                if utf8_file::read(target).map_err(GenerateError::Input)? == generated.spec.source()
                {
                    let fields = &bound.fields;
                    let baseline = Snapshot::capture_selected(&generated.spec, fields)
                        .map_err(GenerateError::Invalid)?;
                    // Rebase the exact published declarations, not the derived
                    // completion table that intentionally omits unobserved hashes.
                    draft::update(
                        &bound.path,
                        fields,
                        baseline.document(),
                        Some(generated.spec.source()),
                    )
                    .map_err(GenerateError::Invalid)?;
                }
            }
        }
        Ok(())
    }

    fn cache(
        &self,
        generated: &Generated,
        resolved_path: &Path,
        package: &str,
        hashes: Option<&BTreeMap<u32, source::Download>>,
        failures: &BTreeMap<u32, source::Error>,
        paths: &mut Vec<PathBuf>,
    ) -> Result<(), GenerateError> {
        let document = resolved_document(self, generated, hashes, failures)?;
        fs_err::create_dir_all(resolved_path.parent().expect("artifact parent")).map_err(
            |source| GenerateError::ArtifactWrite {
                path: resolved_path.to_owned(),
                source,
            },
        )?;
        file_output::write_artifact(resolved_path, document.as_bytes()).map_err(|source| {
            GenerateError::ArtifactWrite {
                path: resolved_path.to_owned(),
                source,
            }
        })?;
        paths.push(resolved_path.to_owned());
        let dir = resolved_path.parent().expect("resolved TOML has a parent");
        let cache_dir = dir.to_path_buf();
        paths.push(
            draft::cache(&cache_dir, package, generated.spec.source())
                .map_err(GenerateError::Invalid)?,
        );
        if let Some(bound) = &self.bound {
            let diff = draft::diff(&bound.source, &bound.original, generated.spec.source())
                .map_err(GenerateError::Invalid)?;
            paths.push(
                draft::save_diff(&cache_dir, package, &diff).map_err(GenerateError::Invalid)?,
            );
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
    written: &'a [PathBuf],
    artifacts: &'a [PathBuf],
    diff: Option<&'a str>,
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
    let mut generation = Generation {
        hashes: (!options.offline).then(BTreeMap::new),
        ..Generation::default()
    };
    let result = generation
        .execute(options)
        .map_err(|error| error.with_written(&generation.written));
    if matches!(options.format, Some(ReportFormat::Toml)) {
        let report = generation.report(options, &result);
        crate::report::write(&mut io::stdout().lock(), &report).map_err(GenerateError::Report)?;
        return Ok(report.success);
    }
    generation.write_human(options, result.as_ref().is_ok_and(|ok| *ok))?;
    result
}

#[derive(Default)]
struct Generation {
    artifacts: Vec<PathBuf>,
    diff: Option<String>,
    manifest_path: Option<PathBuf>,
    baseline_path: Option<PathBuf>,
    manifest_digest: Option<String>,
    hashes: Option<BTreeMap<u32, source::Download>>,
    failures: BTreeMap<u32, source::Error>,
    warnings: Vec<String>,
    candidate: Option<Generated>,
    target: Option<PathBuf>,
    written: Vec<PathBuf>,
}

impl Generation {
    fn execute(&mut self, options: &Options) -> Result<bool, GenerateError> {
        let Self {
            artifacts,
            diff,
            manifest_path,
            baseline_path,
            manifest_digest,
            hashes,
            failures,
            warnings,
            candidate,
            target,
            written,
        } = self;
        let workspace = workspace::discover().map_err(GenerateError::Workspace)?;
        let mut development = workspace
            .existing_development(&options.work)
            .map_err(GenerateError::Workspace)?;
        *manifest_path = Some(development.manifest());
        *baseline_path = if development.spec_authoring() {
            Some(development.spec().map_err(GenerateError::Workspace)?)
        } else {
            None
        };
        let mut input = load(
            &development,
            baseline_path.as_deref(),
            manifest_path,
            manifest_digest,
        )?;
        let resolved_path = development
            .directory()
            .join(".cache")
            .join(format!("{}.resolved.toml", development.package()));
        let selected_target = spec_target(options, &development)?;
        *target = Some(selected_target.clone());
        if options.publishes() {
            protect_target(&input, &selected_target, &resolved_path, &development)?;
        }
        let generated = if input.bound.is_some() {
            from_spec(
                &mut input,
                options,
                hashes,
                failures,
                warnings,
                &development.sources(),
            )?
        } else {
            from_authoring(
                &mut input,
                development.package(),
                options,
                hashes,
                failures,
                warnings,
                &development.sources(),
            )?
        };
        let valid = generated.admissible();
        *candidate = Some(generated);
        input.unchanged()?;
        let generated = candidate.as_ref().expect("generated candidate");
        if options.diff {
            let text = if let Some(bound) = &input.bound
                && options.destination.is_none()
            {
                draft::diff(&bound.source, &bound.original, generated.spec.source())
                    .map_err(GenerateError::Invalid)?
            } else {
                file_output::target_diff(&selected_target, generated.spec.source())
                    .map_err(GenerateError::Output)?
            };
            if matches!(options.format, Some(ReportFormat::Toml)) {
                *diff = Some(text);
            } else {
                io::stdout()
                    .lock()
                    .write_all(text.as_bytes())
                    .map_err(GenerateError::Stdout)?;
            }
        }
        if options.check || (options.diff && !options.apply) {
            return Ok(valid);
        }
        if !valid {
            return Err(GenerateError::CheckFailed);
        }
        if options.stdout {
            io::stdout()
                .lock()
                .write_all(generated.spec.source().as_bytes())
                .map_err(GenerateError::Stdout)?;
            return Ok(true);
        }
        input.publish_spec(
            options,
            &workspace,
            &mut development,
            &selected_target,
            generated,
            written,
        )?;
        if !options.publishes() {
            input.unchanged()?;
        }
        input.cache(
            generated,
            &resolved_path,
            development.package(),
            hashes.as_ref(),
            failures,
            artifacts,
        )?;
        Ok(true)
    }

    fn report<'a>(
        &'a self,
        options: &'a Options,
        result: &Result<bool, GenerateError>,
    ) -> GenerationReport<'a> {
        let success = result.as_ref().is_ok_and(|admissible| *admissible);
        GenerationReport {
            format_version: 5,
            written: &self.written,
            artifacts: &self.artifacts,
            diff: self.diff.as_deref(),
            tool: crate::tool::identity(),
            scope: if self.baseline_path.is_some() {
                "selected-generation-static"
            } else if self.manifest_path.is_some() {
                "manifest-generation-static"
            } else {
                "generation-static"
            },
            valid: self
                .candidate
                .as_ref()
                .and_then(|generated| generated.report.as_ref())
                .map(CheckReport::is_success),
            admissible: self.candidate.as_ref().map(Generated::admissible),
            success,
            baseline_report: self
                .candidate
                .as_ref()
                .and_then(|generated| generated.baseline.as_ref())
                .map(|report| {
                    report.structured(self.baseline_path.as_deref().expect("baseline source"))
                }),
            input: crate::report::Input {
                display_path: self.manifest_path.as_ref().map_or_else(
                    || options.work.as_str().into(),
                    |path| path.to_string_lossy(),
                ),
                sha256: self.manifest_digest.as_deref(),
                revision: None,
            },
            work: &options.work,
            profile: crate::profile::identity(),
            build_contract: self
                .candidate
                .as_ref()
                .and_then(|generated| generated.manifest.as_ref())
                .and_then(|manifest| manifest.build.system.as_deref())
                .and_then(crate::profile::buildsystems::contract_identity),
            report: self
                .candidate
                .as_ref()
                .and_then(|generated| generated.report.as_ref())
                .map(|report| report.structured(self.target.as_deref().expect("candidate target"))),
            source_hashes: self.hashes.as_ref().map(crate::report::Numbered),
            source_hash_failures: crate::report::Numbered(&self.failures),
            authoring_warnings: &self.warnings,
            error: result
                .as_ref()
                .err()
                .map(|error| crate::report::failure(error.code(), error)),
        }
    }

    fn write_human(&self, options: &Options, success: bool) -> Result<(), GenerateError> {
        let warn = |message: std::fmt::Arguments<'_>| {
            output_cli::stderr()
                .message(
                    output_cli::HumanLevel::Warn,
                    Some(Path::new(&options.work)),
                    message,
                )
                .map_err(|error| GenerateError::Stderr(error).with_written(&self.written))
        };
        if success && !options.check && !options.stdout && (!options.diff || options.apply) {
            let mut stderr = output_cli::stderr();
            for path in &self.written {
                stderr
                    .message(
                        output_cli::HumanLevel::Info,
                        None,
                        format_args!("wrote {}", path.display()),
                    )
                    .map_err(GenerateError::Stderr)?;
            }
            if !options.publishes() {
                stderr
                    .message(
                        output_cli::HumanLevel::Info,
                        None,
                        format_args!(
                            "recipe unchanged; apply: ruyipack gen {} --apply",
                            options.work
                        ),
                    )
                    .map_err(GenerateError::Stderr)?;
            }
        }
        for (number, error) in &self.failures {
            warn(format_args!(
                "sources.{number}.sha256: calculation failed: {error}"
            ))?;
        }
        for warning in &self.warnings {
            warn(format_args!("{warning}"))?;
        }
        if let Some(report) = self
            .candidate
            .as_ref()
            .and_then(|candidate| candidate.report.as_ref())
        {
            let subject = PathBuf::from(format!("{} (candidate)", options.work));
            report
                .write_human(&subject, &mut output_cli::stderr())
                .map_err(|error| GenerateError::Stderr(error).with_written(&self.written))?;
        }
        Ok(())
    }
}

fn load(
    development: &workspace::Development,
    baseline_path: Option<&Path>,
    manifest_path: &mut Option<PathBuf>,
    manifest_digest: &mut Option<String>,
) -> Result<Input, GenerateError> {
    let bound = if let Some(expected) = baseline_path {
        let drafts = draft::load(
            manifest_path
                .as_ref()
                .expect("selected input")
                .parent()
                .expect("input parent"),
        )
        .map_err(GenerateError::Invalid)?;
        let [draft]: [draft::Draft; 1] = drafts.try_into().map_err(|_| {
            GenerateError::Invalid("gen WORK requires one saved edit source".into())
        })?;
        if draft.source != expected {
            return Err(GenerateError::Invalid(format!(
                "indexed source {} is not WORK's bound SPEC {}",
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
        draft::read_text(&path).map_err(GenerateError::Invalid)?
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
    cache: &Path,
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
        let mut materials = source::Downloads::new(Some(cache));
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
            .and_then(|url| materials.fetch(source::RemoteSource::parse(&url)?));
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

fn from_spec(
    input: &mut Input,
    options: &Options,
    hashes: &mut Option<BTreeMap<u32, source::Download>>,
    failures: &mut BTreeMap<u32, source::Error>,
    warnings: &mut Vec<String>,
    cache: &Path,
) -> Result<Generated, GenerateError> {
    let bound = input.bound.as_ref().expect("source-bound document");
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
    draft::complete_missing(&mut input.document, snapshot.document());
    // Resolve the edited candidate, not the old source's URL or version.
    let pending = snapshot
        .render_before_hashing(&input.document, &[])
        .map_err(GenerateError::Invalid)?;
    let (resolved, original_sources) = match (
        crate::spec::sources::resolve(&pending, &[]),
        crate::spec::sources::resolve(&parsed, &[]),
    ) {
        (Ok(after), Ok(before)) => (after, before),
        (Err(reason), _) | (_, Err(reason)) => {
            if options.hash {
                return Err(GenerateError::Invalid(reason));
            }
            warnings.push(format!(
                "sources: digest completion skipped: {reason}; existing declarations retained"
            ));
            return finish_spec(&input.document, &snapshot, &parsed, options);
        }
    };
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
    let numbers = spec_downloads(
        &input.document,
        &resolved,
        &changed_urls,
        &explicit_digests,
        options,
        warnings,
    )?;
    for number in &numbers {
        let field = format!("sources.{number}.sha256");
        if !fields.is_empty()
            && !fields
                .iter()
                .any(|selected| selected == &field || field.starts_with(&format!("{selected}.")))
        {
            fields.push(field);
        }
    }
    if fields != bound.fields {
        snapshot = Snapshot::capture_selected(&parsed, &fields).map_err(GenerateError::Invalid)?;
        draft::complete_missing(&mut input.document, snapshot.document());
    }
    if !numbers.is_empty() {
        let downloads = hashes.as_mut().expect("downloads enabled");
        let mut prepared = BTreeMap::new();
        for number in &numbers {
            // Adding digest fields cannot change these already resolved URLs.
            let url = resolved.sources[number]
                .url
                .as_ref()
                .expect("selected resolved URL");
            match source::RemoteSource::parse(url) {
                Ok(url) => {
                    prepared.insert(*number, url);
                }
                Err(error) => {
                    failures.insert(*number, error.at(*number));
                }
            }
        }
        if options.hash && !failures.is_empty() {
            return Err(GenerateError::HashFailed);
        }
        let mut materials = source::Downloads::new(Some(cache));
        for (number, url) in prepared {
            match materials.fetch(url) {
                Ok(download) => {
                    downloads.insert(number, download);
                }
                Err(error) => {
                    failures.insert(number, error.at(number));
                }
            }
        }
        draft::complete_digests(&mut input.document, downloads).map_err(GenerateError::Invalid)?;
        input.unchanged()?;
        if options.hash && !failures.is_empty() {
            return Err(GenerateError::HashFailed);
        }
    }
    for number in &changed_urls {
        if hashes
            .as_ref()
            .is_none_or(|hashes| !hashes.contains_key(number))
        {
            warnings.push(format!("sources.{number}: effective URL changed; any retained digest is a declaration, not verification of the new URL; use --hash to recalculate"));
        }
    }
    let generated = finish_spec(&input.document, &snapshot, &parsed, options)?;
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
    input.bound.as_mut().expect("source-bound document").fields = fields;
    Ok(generated)
}

fn finish_spec(
    document: &Table,
    snapshot: &Snapshot<'_>,
    parsed: &ParsedSpec<'_>,
    options: &Options,
) -> Result<Generated, GenerateError> {
    let generated = crate::spec::candidate::prepare(
        snapshot,
        document,
        &[],
        options.check || options.publishes(),
    )
    .map_err(GenerateError::Invalid)?;
    let baseline = (options.check || options.publishes())
        .then(|| crate::check::analyze(parsed, crate::check::Policy::Authoring, &[]));
    Ok(Generated {
        spec: generated.spec,
        report: generated.report,
        baseline,
        manifest: None,
    })
}

fn spec_downloads(
    document: &Table,
    resolved: &crate::spec::sources::Resolution,
    changed_urls: &[u32],
    explicit_digests: &BTreeSet<u32>,
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<Vec<u32>, GenerateError> {
    let mut numbers = Vec::new();
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
            let supplied =
                crate::spec::document::table::lookup(document, &format!("sources.{number}.sha256"))
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
        }
    }
    Ok(numbers)
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
) -> Result<PathBuf, GenerateError> {
    if options.apply {
        return match development.spec() {
            Ok(path) => Ok(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(development
                .package_directory()
                .join(format!("{}.spec", development.package()))),
            Err(error) => Err(GenerateError::Workspace(error)),
        };
    }
    Ok(match options.destination.as_deref() {
        Some(path) if path == Path::new(".") => development.manifest().with_extension("spec"),
        Some(path) => path.to_owned(),
        None => development
            .directory()
            .join(".cache")
            .join(format!("{}.candidate.spec", development.package())),
    })
}

fn protect_target(
    input: &Input,
    target: &Path,
    resolved_path: &Path,
    development: &workspace::Development,
) -> Result<(), GenerateError> {
    development
        .protect_output(target)
        .map_err(|source| GenerateError::Target {
            path: target.to_owned(),
            source,
        })?;
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
        .is_some_and(|target| target.starts_with(work.join(".state")))
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
    #[error("cannot use SPEC output {}: {source}", .path.display())]
    Target { path: PathBuf, source: io::Error },
    #[error("input path conflicts with generated target {}", .0.display())]
    InputIsTarget(PathBuf),
    #[error("manifest at {} is not for requested package {requested:?}", .path.display())]
    ManifestNotForPackage { requested: String, path: PathBuf },
    #[error("failed to write diagnostics to stderr: {0}")]
    Stderr(#[source] io::Error),
    #[error("generated SPEC failed the selected static checks")]
    CheckFailed,
    #[error("--hash failed for one or more Sources. No derived files written")]
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
            Self::Target { .. } => "output-target",
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
