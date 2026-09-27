// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Typed authoring input and validation before SPEC rendering.

use super::RenderError;
use serde::{Deserialize, Deserializer, de::Error as _};
use std::collections::{BTreeMap, BTreeSet};

/// Deserialized authoring input. Content checks are deferred to `parse` so an
/// empty scaffold reports every unfilled field at once instead of aborting on
/// the first table that fails a semantic check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ManifestInput {
    spec: SpecMetadata,
    package: PackageInput,
    #[serde(deserialize_with = "deserialize_materials")]
    sources: Vec<(u32, Source)>,
    #[serde(default, deserialize_with = "deserialize_materials")]
    patches: Vec<(u32, Patch)>,
    #[serde(default)]
    build: Build,
    build_requires: BuildRequires,
    // Explicit subpackage declarations, keyed by suffix or complete name.
    #[serde(default)]
    subpackages: BTreeMap<String, SubpackageInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct SubpackageInput {
    summary: String,
    description: String,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
    // false: the map key is a suffix appended to the main name (%package <suffix>).
    // true: the key is the complete package name (%package -n <key>).
    #[serde(default)]
    full_name: bool,
    #[serde(default)]
    files: Files,
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
    #[serde(default)]
    vcs: VcsInput,
    // Runtime dependency and capability expressions for the main package.
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
    // The manifest supports BuildArch: noarch, not arbitrary architecture lists.
    #[serde(default)]
    noarch: bool,
    files: Files,
}

/// Authoring input after structural and content checks. RPM expressions are
/// still preserved as text; generation verifies them through the SPEC adapter.
pub(crate) struct Manifest {
    pub(crate) spec: SpecMetadata,
    pub(crate) package: Package,
    pub(crate) sources: Vec<(u32, Source)>,
    pub(crate) patches: Vec<(u32, Patch)>,
    pub(crate) build: Build,
    pub(crate) build_requires: BuildRequires,
    pub(crate) subpackages: Vec<Subpackage>,
}

/// A validated subpackage: its name form plus the same body a main package has.
pub(crate) struct Subpackage {
    pub(crate) name: SubpackageName,
    pub(crate) body: PackageBody,
}

/// How a subpackage names itself relative to the main package.
pub(crate) enum SubpackageName {
    /// Suffix appended to the main name: renders `%package <suffix>`.
    Suffix(String),
    /// Complete package name: renders `%package -n <name>`.
    Absolute(String),
}
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) version: String,
    // This authoring model declares these fields on the main package only;
    // subpackage overrides are not supported.
    pub(crate) license: String,
    pub(crate) url: String,
    pub(crate) vcs: Vcs,
    pub(crate) noarch: bool,
    pub(crate) body: PackageBody,
}

/// Fields shared by the main package and subpackages.
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
    /// No repository fact has been supplied; never means that none exists.
    Unknown,
    Git(String),
    SameAsUrl,
    NoPublicRepository,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct VcsInput {
    git: Option<String>,
    #[serde(default)]
    same_as_url: bool,
    #[serde(default)]
    no_public_repository: bool,
}

/// Resolves the repository declaration; the caller aggregates any error.
fn resolve_vcs(input: &VcsInput) -> Result<Vcs, String> {
    match (input.git.as_deref(), input.same_as_url, input.no_public_repository) {
        (Some(url), false, false) => {
            validate_https_url("package.vcs.git", url)?;
            Ok(Vcs::Git(url.to_owned()))
        }
        (None, false, false) => Ok(Vcs::Unknown),
        (None, true, false) => Ok(Vcs::SameAsUrl),
        (None, false, true) => Ok(Vcs::NoPublicRepository),
        _ => Err(
            "package.vcs: choose exactly one of git, same-as-url = true, or no-public-repository = true"
                .into(),
        ),
    }
}
/// An RPM Source declaration. Local material is not
/// downloaded or assigned a RemoteAsset marker; generation never opens it.
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Source {
    Remote {
        url: String,
        // Missing hashes warn; an explicitly empty hash is invalid.
        #[serde(default)]
        sha256: Option<String>,
    },
    Local {
        path: String,
    },
}

impl Source {
    pub(crate) fn value(&self) -> &str {
        match self {
            Self::Remote { url, .. } => url,
            Self::Local { path } => path,
        }
    }
}
/// Local patch declaration. Keep declaration order for RPM's %autopatch.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Patch {
    pub(crate) path: String,
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
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Files {
    #[serde(default)]
    pub(crate) license: Vec<String>,
    #[serde(default)]
    pub(crate) doc: Vec<String>,
    // Serde-optional so a subpackage may declare no files; the main package's
    // "at least one entry" requirement is enforced in validate_body instead.
    #[serde(default)]
    pub(crate) entries: Vec<String>,
    /// Files generated during the build, passed to RPM as repeated `%files -f`.
    #[serde(default)]
    pub(crate) lists: Vec<String>,
}

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
    if let Err(error) = validate_single_line(&format!("{prefix}.summary"), summary) {
        messages.push(error);
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
        if let Err(error) = validate_single_line(&format!("{prefix}.requires"), require) {
            messages.push(error);
        }
    }
    for provide in provides {
        if let Err(error) = validate_single_line(&format!("{prefix}.provides"), provide) {
            messages.push(error);
        }
    }
    if files_required
        && files.license.is_empty()
        && files.doc.is_empty()
        && files.entries.is_empty()
        && files.lists.is_empty()
    {
        messages.push(invalid(
            &format!("{prefix}.files"),
            "at least one file entry is required",
        ));
    }
    for (suffix, values) in [
        ("files.license", &files.license),
        ("files.doc", &files.doc),
        ("files.lists", &files.lists),
    ] {
        let field = format!("{prefix}.{suffix}");
        for value in values {
            if let Err(error) = validate_single_line(&field, value) {
                messages.push(error);
            }
            if value.chars().any(char::is_whitespace) {
                messages.push(invalid(
                    &field,
                    "expected one path per entry, without whitespace",
                ));
            }
        }
    }
    for path in &files.lists {
        if path.starts_with('-') || path.starts_with('#') {
            messages.push(invalid(
                &format!("{prefix}.files.lists"),
                "expected a file-list path, not an option or comment",
            ));
        }
    }
    for entry in &files.entries {
        if let Err(error) = validate_single_line(&format!("{prefix}.files.entries"), entry) {
            messages.push(error);
        } else if let Err(error) = crate::spec::files::entry(entry) {
            messages.push(invalid(&format!("{prefix}.files.entries"), error));
        }
    }
    messages
}

/// Reads authoring fields without evaluating RPM macros. Structural errors
/// abort deserialization; content errors are collected across the manifest.
pub(crate) fn parse(source: &str) -> Result<Manifest, RenderError> {
    let mut input: ManifestInput = toml::from_str(source)?;
    // Source identity is numeric; Patch declaration order is also application
    // order in native RPM's %autopatch, so never sort patches.
    input.sources.sort_by_key(|(number, _)| *number);
    let package = &input.package;
    let invalid = |field: &str, reason: &str| format!("{field}: {reason}");

    let mut errors = Vec::new();
    let mut record = |result: Result<(), String>| {
        if let Err(error) = result {
            errors.push(error);
        }
    };

    record(
        crate::spec_metadata::validate_years(&input.spec.copyright_years).map_err(str::to_owned),
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
    record(crate::check::metadata::Field::Name.validate(&package.name));
    record(crate::check::metadata::Field::Version.validate(&package.version));
    record(validate_single_line("package.license", &package.license));
    record(validate_https_url("package.url", &package.url));
    // Deferred from deserialization so an empty [package.vcs] table joins the
    // report instead of aborting the parse before other fields are seen.
    let vcs = match resolve_vcs(&package.vcs) {
        Ok(vcs) => Some(vcs),
        Err(error) => {
            record(Err(error));
            None
        }
    };
    if !input.sources.iter().any(|(number, _)| *number == 0) {
        record(Err(invalid("sources", "sources.0 is required")));
    }
    for (number, source) in &input.sources {
        let field = format!("sources.{number}");
        match source {
            Source::Remote { url, sha256 } => {
                record(validate_source_url(&format!("{field}.url"), url, package));
                if let Some(hash) = sha256 {
                    record(
                        crate::source::validate_sha256(hash)
                            .map_err(|reason| invalid(&format!("{field}.sha256"), reason)),
                    );
                }
            }
            Source::Local { path } => record(validate_local_path(&format!("{field}.path"), path)),
        }
    }
    for (number, patch) in &input.patches {
        record(validate_local_path(
            &format!("patches.{number}.path"),
            &patch.path,
        ));
    }
    if let Some(system) = &input.build.system
        && crate::profile::buildsystems::contract(system).is_none()
    {
        record(Err(invalid(
            "build.system",
            &format!("unsupported build system {system:?}"),
        )));
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
            record(validate_single_line(
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
        record(validate_single_line("build-requires.rpm", requirement));
    }
    errors.extend(validate_body(
        "package",
        &package.summary,
        &package.description,
        &package.requires,
        &package.provides,
        &package.files,
        true,
    ));
    let mut package_names = BTreeSet::from([package.name.clone()]);
    for (name, subpackage) in &input.subpackages {
        let field = format!("subpackages.{name}");
        if let Err(message) = crate::check::metadata::Field::Name.validate_at(name, &field) {
            errors.push(message);
        }
        let effective_name = if subpackage.full_name {
            name.clone()
        } else {
            format!("{}-{name}", package.name)
        };
        // Distinct table keys can still designate the same RPM package.
        if !package_names.insert(effective_name.clone()) {
            errors.push(invalid(
                &field,
                &format!("duplicate package name {effective_name:?}"),
            ));
        }
        errors.extend(validate_body(
            &field,
            &subpackage.summary,
            &subpackage.description,
            &subpackage.requires,
            &subpackage.provides,
            &subpackage.files,
            false,
        ));
    }

    if !errors.is_empty() {
        return Err(RenderError::Invalid(errors.join("\n")));
    }
    let vcs = vcs.expect("vcs is set when no errors were recorded");
    let subpackages = input
        .subpackages
        .into_iter()
        .map(|(name, sub)| Subpackage {
            name: if sub.full_name {
                SubpackageName::Absolute(name)
            } else {
                SubpackageName::Suffix(name)
            },
            body: PackageBody {
                summary: sub.summary,
                description: sub.description,
                requires: sub.requires,
                provides: sub.provides,
                files: sub.files,
            },
        })
        .collect();
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
        patches: input.patches,
        build: input.build,
        build_requires: input.build_requires,
        subpackages,
    })
}

/// Reads numeric source keys without silently merging alternate spellings.
fn deserialize_materials<'de, D, T>(deserializer: D) -> Result<Vec<(u32, T)>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    // TOML preserve_order retains the author's Patch sequence. A sorted map
    // would silently reorder %autopatch even though the numbers stayed intact.
    let entries = toml::Table::deserialize(deserializer)?;
    let mut seen = BTreeSet::new();
    let mut materials = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let number = key.parse::<u32>().map_err(|_| {
            D::Error::custom(format!("{key}: expected a non-negative material number"))
        })?;
        if !seen.insert(number) {
            return Err(D::Error::custom(format!(
                "duplicate material number {number}"
            )));
        }
        materials.push((number, value.try_into().map_err(D::Error::custom)?));
    }
    Ok(materials)
}

fn validate_local_path(field: &str, path: &str) -> Result<(), String> {
    validate_single_line(field, path)?;
    // Declares an RPM input; no local file is opened during generation.
    if path.chars().any(char::is_whitespace)
        || path.contains("://")
        || path.starts_with(['/', '-', '#'])
        || path.split('/').any(|p| p == "..")
    {
        return Err(format!(
            "{field}: expected a relative material path without whitespace, URL, or parent traversal"
        ));
    }
    Ok(())
}

fn validate_single_line(field: &str, value: &str) -> Result<(), String> {
    crate::spec_metadata::validate_single_line(value).map_err(|reason| format!("{field}: {reason}"))
}

/// Generation has an HTTPS-only policy; editing existing HTTP sources is supported.
fn validate_source_url(field: &str, value: &str, package: &PackageInput) -> Result<(), String> {
    validate_single_line(field, value)?;
    let url = crate::source::validate_expression(
        value,
        &[
            ("name", &package.name),
            ("version", &package.version),
            ("url", &package.url),
        ],
    )
    .map_err(|reason| format!("{field}: {reason}"))?;
    crate::source::require_https(field, url)
}

fn validate_https_url(field: &str, value: &str) -> Result<(), String> {
    validate_single_line(field, value)?;
    let url = crate::source::validate_authoring_url(value)
        .map_err(|reason| format!("{field}: {reason}"))?;
    crate::source::require_https(field, url)
}
