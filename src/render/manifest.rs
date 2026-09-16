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
pub(super) struct Manifest {
    pub(super) spec: SpecMetadata,
    pub(super) package: Package,
    #[serde(deserialize_with = "read_sources")]
    pub(super) sources: BTreeMap<u32, Source>,
    pub(super) build: Build,
    pub(super) build_requires: BuildRequires,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct SpecMetadata {
    pub(super) copyright_years: String,
    pub(super) contributors: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Package {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) summary: String,
    pub(super) license: String,
    pub(super) url: String,
    pub(super) description: String,
    pub(super) vcs: Vcs,
    pub(super) files: Files,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(super) struct Vcs {
    pub(super) no_public_repository: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    pub(super) url: String,
    pub(super) sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Build {
    pub(super) system: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BuildRequires {
    pub(super) rpm: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Files {
    #[serde(default)]
    pub(super) license: Vec<String>,
    #[serde(default)]
    pub(super) doc: Vec<String>,
    pub(super) entries: Vec<String>,
}

/// Reads the supported authoring fields without evaluating RPM macros.
pub(super) fn parse(source: &str) -> Result<Manifest, RenderError> {
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
    if !package
        .name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        || !package
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
    {
        return Err(invalid("package.name", "expected an RPM package name"));
    }
    if package.version.is_empty()
        || !package
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+~^".contains(&b))
    {
        return Err(invalid("package.version", "expected a literal RPM version"));
    }
    single_line("package.summary", &package.summary)?;
    single_line("package.license", &package.license)?;
    https_url("package.url", &package.url)?;
    if !package.vcs.no_public_repository {
        return Err(invalid(
            "package.vcs",
            "this renderer requires no-public-repository = true",
        ));
    }
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
        return Err(invalid(
            "sources",
            "sources.0 is required for default unpacking",
        ));
    }
    for (number, source) in &manifest.sources {
        source_url(&format!("sources.{number}.url"), &source.url, package)?;
        crate::source::validate_sha256(&source.sha256)
            .map_err(|reason| invalid(&format!("sources.{number}.sha256"), reason))?;
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
            "{field}: new openRuyi sources require HTTPS"
        )));
    }
    Ok(())
}
