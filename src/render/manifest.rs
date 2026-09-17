// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Typed authoring input and validation before SPEC rendering.

use super::RenderError;
use serde::{Deserialize, Deserializer, de::Error as _};
use std::collections::BTreeMap;

/// Deserialized authoring input. Content checks are deferred to `parse` so an
/// empty scaffold reports every unfilled field at once instead of aborting on
/// the first table that fails a semantic check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ManifestInput {
    spec: SpecMetadata,
    package: PackageInput,
    #[serde(deserialize_with = "read_sources")]
    sources: BTreeMap<u32, Source>,
    #[serde(default)]
    build: Build,
    build_requires: BuildRequires,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageInput {
    name: String,
    version: String,
    summary: String,
    license: String,
    url: String,
    description: String,
    vcs: VcsInput,
    // Runtime dependency and capability expressions for the main package.
    // Subpackages are a separate, later concern; these are the main package's.
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
    // openRuyi only ever uses BuildArch: noarch, so this is a flag, not a free
    // architecture list. Other BuildArch values are a documented TODO.
    #[serde(default)]
    noarch: bool,
    files: Files,
}

/// Validated manifest. It only exists once `parse` accepts every field, so
/// downstream rendering and verification never see an unresolved value.
pub(crate) struct Manifest {
    pub(crate) spec: SpecMetadata,
    pub(crate) package: Package,
    pub(crate) sources: BTreeMap<u32, Source>,
    pub(crate) build: Build,
    pub(crate) build_requires: BuildRequires,
}
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) version: String,
    // License, URL, VCS, and noarch belong to the main package alone; a
    // subpackage inherits them and never restates them.
    pub(crate) license: String,
    pub(crate) url: String,
    pub(crate) vcs: Vcs,
    pub(crate) noarch: bool,
    // Fields a subpackage also carries live in the shared body, so the renderer
    // and verifier can treat main package and subpackage through one path.
    pub(crate) body: PackageBody,
}

/// The part of a package shared by the main package and every subpackage:
/// its summary, description, dependency edges, and file list.
pub(crate) struct PackageBody {
    pub(crate) summary: String,
    pub(crate) description: String,
    pub(crate) requires: Vec<String>,
    pub(crate) provides: Vec<String>,
    pub(crate) files: Files,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct SpecMetadata {
    pub(crate) copyright_years: String,
    pub(crate) contributors: Vec<String>,
}
pub(crate) enum Vcs {
    Git(String),
    SameAsUrl,
    NoPublicRepository,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct VcsInput {
    git: Option<String>,
    #[serde(default)]
    same_as_url: bool,
    #[serde(default)]
    no_public_repository: bool,
}

/// Resolves the repository declaration; the caller aggregates any error.
fn resolve_vcs(input: &VcsInput) -> Result<Vcs, RenderError> {
    match (input.git.as_deref(), input.same_as_url, input.no_public_repository) {
        (Some(url), false, false) => {
            https_url("package.vcs.git", url)?;
            Ok(Vcs::Git(url.to_owned()))
        }
        (None, true, false) => Ok(Vcs::SameAsUrl),
        (None, false, true) => Ok(Vcs::NoPublicRepository),
        _ => Err(RenderError::Invalid(
            "package.vcs: choose exactly one of git, same-as-url = true, or no-public-repository = true"
                .into(),
        )),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub(crate) url: String,
    // Absent means a bare #!RemoteAsset with no digest, which openRuyi accepts.
    // An empty string is still rejected, so a blank scaffold field is not a
    // silent opt-in to the bare form.
    #[serde(default)]
    pub(crate) sha256: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Build {
    pub(crate) system: Option<String>,
    #[serde(default)]
    pub(crate) stages: BTreeMap<Stage, StageConfig>,
}

#[derive(Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Stage {
    Prep,
    Conf,
    Build,
    Install,
    Check,
}

impl Stage {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Prep => "prep",
            Self::Conf => "conf",
            Self::Build => "build",
            Self::Install => "install",
            Self::Check => "check",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StageConfig {
    #[serde(default)]
    pub(crate) options: Vec<String>,
    #[serde(default)]
    pub(crate) prepend: String,
    /// Sets the main section, overriding any build-system action. An empty string emits an empty section.
    pub(crate) replace: Option<String>,
    #[serde(default)]
    pub(crate) append: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildRequires {
    pub(crate) rpm: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Files {
    #[serde(default)]
    pub(crate) license: Vec<String>,
    #[serde(default)]
    pub(crate) doc: Vec<String>,
    pub(crate) entries: Vec<String>,
}

/// Reads the supported authoring fields without evaluating RPM macros.
///
/// Structural problems (missing fields, wrong types, unknown keys, malformed
/// source numbers) still abort during deserialization. Every content check
/// then runs on the whole manifest and the failures are reported together, so
/// filling one field does not just uncover the next.
/// Validates the fields shared by the main package and every subpackage,
/// returning one formatted message per problem. `prefix` names the location in
/// reports (e.g. `package` or `subpackages.devel`). `files_required` is true for
/// the main package and false for subpackages, which may ship no files.
fn validate_body(
    prefix: &str,
    summary: &str,
    description: &str,
    requires: &[String],
    provides: &[String],
    files: &Files,
    files_required: bool,
) -> Vec<String> {
    let invalid = |field: &str, reason: &str| format!("{field}: {reason}");
    let mut messages = Vec::new();
    if let Err(error) = single_line(&format!("{prefix}.summary"), summary) {
        messages.push(error.to_string());
    }
    if description.trim().is_empty()
        || description.chars().any(|c| c.is_control() && c != '\n')
        || description
            .lines()
            .any(|line| line.trim_start().starts_with('%'))
    {
        messages.push(invalid(
            &format!("{prefix}.description"),
            "expected non-empty LF text without lines starting with %",
        ));
    }
    for require in requires {
        if let Err(error) = single_line(&format!("{prefix}.requires"), require) {
            messages.push(error.to_string());
        }
    }
    for provide in provides {
        if let Err(error) = single_line(&format!("{prefix}.provides"), provide) {
            messages.push(error.to_string());
        }
    }
    if files_required
        && files.license.is_empty()
        && files.doc.is_empty()
        && files.entries.is_empty()
    {
        messages.push(invalid(
            &format!("{prefix}.files"),
            "at least one file entry is required",
        ));
    }
    for (suffix, values) in [
        ("files.license", &files.license),
        ("files.doc", &files.doc),
        ("files.entries", &files.entries),
    ] {
        let field = format!("{prefix}.{suffix}");
        for value in values {
            if let Err(error) = single_line(&field, value) {
                messages.push(error.to_string());
            }
            if value.chars().any(char::is_whitespace) {
                messages.push(invalid(
                    &field,
                    "expected one path per entry, without whitespace",
                ));
            }
        }
    }
    for entry in &files.entries {
        if !entry.starts_with('/') && !entry.starts_with("%{") {
            messages.push(invalid(
                &format!("{prefix}.files.entries"),
                "paths must start with / or %{",
            ));
        }
    }
    messages
}

pub(crate) fn parse(source: &str) -> Result<Manifest, RenderError> {
    let input: ManifestInput = toml::from_str(source)?;
    let package = &input.package;
    let invalid = |field: &str, reason: &str| RenderError::Invalid(format!("{field}: {reason}"));

    let mut errors = Vec::new();
    let mut record = |result: Result<(), RenderError>| {
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    };

    record(
        crate::spec_metadata::validate_years(&input.spec.copyright_years)
            .map_err(|error| RenderError::Invalid(error.into())),
    );
    if input.spec.contributors.is_empty() {
        record(Err(invalid(
            "spec.contributors",
            "at least one contributor is required",
        )));
    }
    for contributor in &input.spec.contributors {
        record(
            crate::spec_metadata::validate_contributor(contributor)
                .map_err(|reason| invalid("spec.contributors", reason)),
        );
    }
    record(
        crate::check::metadata::Field::Name
            .validate(&package.name)
            .map_err(RenderError::Invalid),
    );
    record(
        crate::check::metadata::Field::Version
            .validate(&package.version)
            .map_err(RenderError::Invalid),
    );
    record(single_line("package.license", &package.license));
    record(https_url("package.url", &package.url));
    // Deferred from deserialization so an empty [package.vcs] table joins the
    // report instead of aborting the parse before other fields are seen.
    let vcs = match resolve_vcs(&package.vcs) {
        Ok(vcs) => Some(vcs),
        Err(error) => {
            record(Err(error));
            None
        }
    };
    if !input.sources.contains_key(&0) {
        record(Err(invalid("sources", "sources.0 is required")));
    }
    for (number, source) in &input.sources {
        record(source_url(
            &format!("sources.{number}.url"),
            &source.url,
            package,
        ));
        if let Some(sha256) = &source.sha256 {
            record(
                crate::source::validate_sha256(sha256)
                    .map_err(|reason| invalid(&format!("sources.{number}.sha256"), reason)),
            );
        }
    }
    for (stage, config) in &input.build.stages {
        if input.build.system.is_none() && !config.options.is_empty() {
            record(Err(invalid(
                &format!("build.stages.{}.options", stage.as_str()),
                "requires build.system; put arguments in the explicit stage script",
            )));
        }
        if config.replace.is_some() && !config.options.is_empty() {
            record(Err(invalid(
                &format!("build.stages.{}", stage.as_str()),
                "options cannot be combined with replace; put arguments in the replacement script",
            )));
        }
        for option in &config.options {
            record(single_line(
                &format!("build.stages.{}.options", stage.as_str()),
                option,
            ));
        }
        for (name, script) in [
            ("prepend", Some(config.prepend.as_str())),
            ("replace", config.replace.as_deref()),
            ("append", Some(config.append.as_str())),
        ] {
            let Some(script) = script else {
                continue;
            };
            if script
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                record(Err(invalid(
                    &format!("build.stages.{}.{name}", stage.as_str()),
                    "expected script text using LF line endings without control characters other than tabs",
                )));
            }
        }
    }
    for requirement in &input.build_requires.rpm {
        record(single_line("build-requires.rpm", requirement));
    }
    // summary, description, requires, provides, and files share one validator
    // so a subpackage is held to the same rules as the main package. Only the
    // field prefix and whether files may be empty differ.
    for message in validate_body(
        "package",
        &package.summary,
        &package.description,
        &package.requires,
        &package.provides,
        &package.files,
        true,
    ) {
        errors.push(message);
    }

    if !errors.is_empty() {
        return Err(RenderError::Invalid(errors.join("\n")));
    }
    // Every field passed, so the deferred repository choice resolved.
    let vcs = vcs.expect("vcs is set when no errors were recorded");
    Ok(Manifest {
        package: Package {
            name: input.package.name,
            version: input.package.version,
            license: input.package.license,
            url: input.package.url,
            vcs,
            noarch: input.package.noarch,
            body: PackageBody {
                summary: input.package.summary,
                description: input.package.description,
                requires: input.package.requires,
                provides: input.package.provides,
                files: input.package.files,
            },
        },
        spec: input.spec,
        sources: input.sources,
        build: input.build,
        build_requires: input.build_requires,
    })
}

/// Reads numeric source keys without silently merging alternate spellings.
fn read_sources<'de, D>(deserializer: D) -> Result<BTreeMap<u32, Source>, D::Error>
where
    D: Deserializer<'de>,
{
    let entries = BTreeMap::<String, Source>::deserialize(deserializer)?;
    let mut sources = BTreeMap::new();
    for (key, source) in entries {
        let number = key.parse::<u32>().map_err(|_| {
            D::Error::custom(format!(
                "sources.{key}: expected a non-negative source number"
            ))
        })?;
        if sources.insert(number, source).is_some() {
            return Err(D::Error::custom(format!(
                "duplicate source number {number}"
            )));
        }
    }
    Ok(sources)
}

fn single_line(field: &str, value: &str) -> Result<(), RenderError> {
    crate::spec_metadata::validate_single_line(value)
        .map_err(|reason| RenderError::Invalid(format!("{field}: {reason}")))
}

/// Generation has an HTTPS-only policy; editing existing HTTP sources is supported.
fn source_url(field: &str, value: &str, package: &PackageInput) -> Result<(), RenderError> {
    single_line(field, value)?;
    let scheme = crate::source::validate_expression(
        value,
        &[
            ("name", &package.name),
            ("version", &package.version),
            ("url", &package.url),
        ],
    )
    .map_err(|reason| RenderError::Invalid(format!("{field}: {reason}")))?;
    require_https(field, scheme)
}

fn https_url(field: &str, value: &str) -> Result<(), RenderError> {
    single_line(field, value)?;
    let scheme = crate::source::validate_url(value)
        .map_err(|reason| RenderError::Invalid(format!("{field}: {reason}")))?;
    require_https(field, scheme)
}

fn require_https(field: &str, scheme: crate::source::Scheme) -> Result<(), RenderError> {
    if scheme != crate::source::Scheme::Https {
        return Err(RenderError::Invalid(format!(
            "{field}: expected an HTTPS URL"
        )));
    }
    Ok(())
}
