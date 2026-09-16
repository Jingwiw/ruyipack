// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Typed authoring input and validation before SPEC rendering.

use super::RenderError;
use serde::{Deserialize, Deserializer, de::Error as _};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Manifest {
    pub(crate) spec: SpecMetadata,
    pub(crate) package: Package,
    #[serde(deserialize_with = "read_sources")]
    pub(crate) sources: BTreeMap<u32, Source>,
    #[serde(default)]
    pub(crate) build: Build,
    pub(crate) build_requires: BuildRequires,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct SpecMetadata {
    pub(crate) copyright_years: String,
    pub(crate) contributors: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) summary: String,
    pub(crate) license: String,
    pub(crate) url: String,
    pub(crate) description: String,
    pub(crate) vcs: Vcs,
    pub(crate) files: Files,
}
#[derive(Deserialize)]
#[serde(try_from = "VcsInput")]
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

impl TryFrom<VcsInput> for Vcs {
    type Error = RenderError;

    fn try_from(input: VcsInput) -> Result<Self, Self::Error> {
        match (input.git, input.same_as_url, input.no_public_repository) {
            (Some(url), false, false) => {
                https_url("package.vcs.git", &url)?;
                Ok(Self::Git(url))
            }
            (None, true, false) => Ok(Self::SameAsUrl),
            (None, false, true) => Ok(Self::NoPublicRepository),
            _ => Err(RenderError::Invalid(
                "package.vcs: choose exactly one of git, same-as-url = true, or no-public-repository = true"
                    .into(),
            )),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub(crate) url: String,
    pub(crate) sha256: String,
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
pub(crate) fn parse(source: &str) -> Result<Manifest, RenderError> {
    let manifest: Manifest = toml::from_str(source)?;
    let package = &manifest.package;
    let invalid = |field: &str, reason: &str| RenderError::Invalid(format!("{field}: {reason}"));
    crate::spec_metadata::validate_years(&manifest.spec.copyright_years)
        .map_err(|error| RenderError::Invalid(error.into()))?;
    if manifest.spec.contributors.is_empty() {
        return Err(invalid(
            "spec.contributors",
            "at least one contributor is required",
        ));
    }
    for contributor in &manifest.spec.contributors {
        single_line("spec.contributors", contributor)?;
        if contributor.contains('%') {
            return Err(invalid(
                "spec.contributors",
                "RPM macros are not allowed in the header",
            ));
        }
    }
    crate::check::metadata::Field::Name
        .validate(&package.name)
        .map_err(RenderError::Invalid)?;
    crate::check::metadata::Field::Version
        .validate(&package.version)
        .map_err(RenderError::Invalid)?;
    single_line("package.summary", &package.summary)?;
    single_line("package.license", &package.license)?;
    https_url("package.url", &package.url)?;
    if package.description.trim().is_empty()
        || package
            .description
            .chars()
            .any(|c| c.is_control() && c != '\n')
        || package
            .description
            .lines()
            .any(|line| line.trim_start().starts_with('%'))
    {
        return Err(invalid(
            "package.description",
            "expected non-empty LF text without lines starting with %",
        ));
    }
    if !manifest.sources.contains_key(&0) {
        return Err(invalid("sources", "sources.0 is required"));
    }
    for (number, source) in &manifest.sources {
        source_url(&format!("sources.{number}.url"), &source.url, package)?;
        crate::source::validate_sha256(&source.sha256)
            .map_err(|reason| invalid(&format!("sources.{number}.sha256"), reason))?;
    }
    for (stage, config) in &manifest.build.stages {
        if manifest.build.system.is_none() && !config.options.is_empty() {
            return Err(invalid(
                &format!("build.stages.{}.options", stage.as_str()),
                "requires build.system; put arguments in the explicit stage script",
            ));
        }
        if config.replace.is_some() && !config.options.is_empty() {
            return Err(invalid(
                &format!("build.stages.{}", stage.as_str()),
                "options cannot be combined with replace; put arguments in the replacement script",
            ));
        }
        for option in &config.options {
            single_line(&format!("build.stages.{}.options", stage.as_str()), option)?;
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
                return Err(invalid(
                    &format!("build.stages.{}.{name}", stage.as_str()),
                    "expected script text using LF line endings without control characters other than tabs",
                ));
            }
        }
    }
    for requirement in &manifest.build_requires.rpm {
        single_line("build-requires.rpm", requirement)?;
    }
    let files = &package.files;
    if files.license.is_empty() && files.doc.is_empty() && files.entries.is_empty() {
        return Err(invalid(
            "package.files",
            "at least one file entry is required",
        ));
    }
    for (field, values) in [
        ("package.files.license", &files.license),
        ("package.files.doc", &files.doc),
        ("package.files.entries", &files.entries),
    ] {
        for value in values {
            single_line(field, value)?;
            if value.chars().any(char::is_whitespace) {
                return Err(invalid(
                    field,
                    "expected one path per entry, without whitespace",
                ));
            }
        }
    }
    for entry in &files.entries {
        if !entry.starts_with('/') && !entry.starts_with("%{") {
            return Err(invalid(
                "package.files.entries",
                "paths must start with / or %{",
            ));
        }
    }
    Ok(manifest)
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
    if value.is_empty()
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.ends_with('\\')
    {
        return Err(RenderError::Invalid(format!(
            "{field}: expected non-empty single-line text without edge whitespace or a trailing backslash"
        )));
    }
    Ok(())
}

/// Generation has an HTTPS-only policy; editing existing HTTP sources is supported.
fn source_url(field: &str, value: &str, package: &Package) -> Result<(), RenderError> {
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
