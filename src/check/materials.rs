// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared material inventory and checksum-verified preparation for check, fetch and build.

use crate::{source, spec};
use fs_err as fs;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::Path,
};

use crate::report::{Failure, failure};

use crate::file_digest::{self, Content};

#[derive(Serialize)]
struct Record {
    identity: String,
    expression: String,
    resolved: Option<String>,
    path: Option<String>,
    declared_sha256: Option<String>,
    #[serde(flatten)]
    outcome: Outcome,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
enum Outcome {
    Ready {
        content: Content,
    },
    Error {
        content: Option<Content>,
        error: Failure,
    },
}

fn filename(value: &str) -> Result<&str, Failure> {
    spec::sources::filename(value).map_err(|e| failure("invalid-filename", e))
}

#[derive(Serialize)]
pub(crate) struct Report {
    pub(crate) valid: bool,
    source_dir: Option<String>,
    files: Vec<Record>,
    error: Option<Failure>,
}

/// Checks only declared inputs in a stable, prepared _sourcedir. No fetching or writes.
pub(crate) fn analyze(
    source_dir: Option<&Path>,
    cache: Option<&Path>,
    parsed: &spec::ParsedSpec<'_>,
    defines: &[String],
) -> Report {
    let mut directory = None;
    let mut records = Vec::new();
    let result = (|| {
        let source_dir = source_dir.ok_or_else(|| failure(
            "source-directory",
            "this committed SPEC has no local materials; provide --source-dir with its prepared materials",
        ))?;
        let root = fs::canonicalize(source_dir).map_err(|e| failure("source-directory", e))?;
        if !root.is_dir() {
            return Err(failure(
                "source-directory",
                format!("{}: expected a directory", root.display()),
            ));
        }
        directory = Some(root.to_string_lossy().into_owned());
        let declarations = declarations(parsed, defines)?;
        records.extend(
            declarations
                .entries
                .into_iter()
                .map(|(identity, material)| {
                    check_material(&root, cache, &declarations.collisions, identity, material)
                }),
        );
        if let Some(reason) = declarations.incomplete {
            return Err(failure("material-resolution", reason));
        }
        Ok(())
    })();
    let valid = result.is_ok()
        && records
            .iter()
            .all(|r| matches!(r.outcome, Outcome::Ready { .. }));
    Report {
        valid,
        source_dir: directory,
        files: records,
        error: result.err(),
    }
}

struct Declarations {
    entries: Vec<(String, spec::sources::Source)>,
    collisions: BTreeMap<String, String>,
    incomplete: Option<String>,
}

fn declarations(
    parsed: &spec::ParsedSpec<'_>,
    defines: &[String],
) -> Result<Declarations, Failure> {
    let resolved = spec::sources::resolve_materials(parsed, defines)
        .map_err(|e| failure("material-resolution", e))?;
    let mut declarations = resolved
        .sources
        .into_iter()
        .map(|(n, s)| (format!("Source{n}"), s))
        .chain(
            resolved
                .patches
                .into_iter()
                .map(|(n, s)| (format!("Patch{n}"), s)),
        )
        .collect::<Vec<_>>();
    declarations.sort_by_key(|(_, s)| s.span.bytes.start);
    let mut names: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for (identity, material) in &declarations {
        if let Ok(value) = &material.url
            && let Ok(name) = filename(value)
        {
            names.entry(name).or_default().push((identity, value));
        }
    }
    // Identical declarations may reuse one file. Different locations targeting
    // that filename are ambiguous, even when the staged bytes happen to match.
    let collisions = names
        .into_iter()
        .filter(|(_, peers)| peers.iter().any(|(_, value)| *value != peers[0].1))
        .map(|(name, peers)| {
            (
                name.to_owned(),
                format!(
                    "{name} is targeted by {}",
                    peers
                        .iter()
                        .map(|(identity, _)| *identity)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(Declarations {
        entries: declarations,
        collisions,
        incomplete: resolved.incomplete,
    })
}

/// Requirements for a delivery plan, without downloading or reading a material directory.
pub(crate) struct Requirement {
    pub(crate) identity: String,
    pub(crate) name: String,
    pub(crate) remote: bool,
    pub(crate) sha256: Option<String>,
}

pub(crate) fn requirements(parsed: &spec::ParsedSpec<'_>) -> Result<Vec<Requirement>, String> {
    let declarations = declarations(parsed, &[]).map_err(|e| e.message)?;
    if let Some(reason) = declarations.incomplete {
        return Err(reason);
    }
    declarations
        .entries
        .into_iter()
        .map(|(identity, material)| {
            let (value, name, digest) =
                material_name(&material).map_err(|e| format!("{identity}: {}", e.message))?;
            if let Some(message) = declarations.collisions.get(name) {
                return Err(message.clone());
            }
            Ok(Requirement {
                identity,
                name: name.to_owned(),
                remote: source::is_remote_url(value),
                sha256: digest.cloned(),
            })
        })
        .collect()
}

fn material_name(
    material: &spec::sources::Source,
) -> Result<(&str, &str, Option<&String>), Failure> {
    let value = material
        .url
        .as_ref()
        .map_err(|e| failure("unresolved", e))?;
    let name = filename(value)?;
    let digest = material
        .digest
        .as_ref()
        .map_err(|e| failure("invalid-digest", e))?;
    if let Some(hash) = digest {
        source::validate_sha256(hash).map_err(|e| failure("invalid-digest", e))?;
    }
    Ok((value, name, digest.as_ref()))
}

/// Build preparation reuses inventory facts; offline `check` remains read-only.
/// Preflight every declaration before fetching any missing remote material.
pub(crate) fn prepare(
    root: &Path,
    cache: &Path,
    parsed: &spec::ParsedSpec<'_>,
    defines: &[String],
    offline: bool,
) -> Result<Report, String> {
    let report = analyze(Some(root), Some(cache), parsed, defines);
    if let Some(error) = &report.error {
        return Err(error.message.clone());
    }
    let mut pending: BTreeMap<_, (source::RemoteSource<'_>, &str)> = BTreeMap::new();
    for row in &report.files {
        let remote = row
            .resolved
            .as_deref()
            .map(source::RemoteSource::classify)
            .transpose()
            .map_err(|e| format!("{}: {e}", row.identity))?
            .flatten();
        if remote.is_some() && row.declared_sha256.is_none() {
            return Err(format!(
                "{}: remote material requires a declared SHA-256. For authoring TOML, run gen WORK --apply; for SPEC edits, run edit WORK --hash --apply",
                row.identity
            ));
        }
        match &row.outcome {
            Outcome::Ready { .. } => {}
            Outcome::Error { error, .. }
                if error.code == "missing-file" && remote.is_some() && !offline =>
            {
                let path = cache.join(
                    filename(row.resolved.as_deref().expect("resolved remote"))
                        .map_err(|e| e.message)?,
                );
                let hash = row
                    .declared_sha256
                    .as_deref()
                    .expect("remote digest checked");
                if let Some((_, previous)) = pending.get(&path)
                    && !hash.eq_ignore_ascii_case(previous)
                {
                    return Err(format!(
                        "{}: conflicting SHA-256 declarations",
                        path.display()
                    ));
                }
                pending.insert(path, (remote.expect("remote checked"), hash));
            }
            Outcome::Error { error, .. } => {
                return Err(format!(
                    "{}: {}{}",
                    row.identity,
                    error.message,
                    if offline && error.code == "missing-file" {
                        "; stage this file before retrying offline"
                    } else {
                        ""
                    }
                ));
            }
        }
    }
    if !pending.is_empty() {
        fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    }
    for (path, (remote, expected)) in pending {
        eprintln!("source: downloading {}", path.display());
        remote
            .download_to(&path, expected)
            .map_err(|e| e.to_string())?;
    }
    let report = analyze(Some(root), Some(cache), parsed, defines);
    if !report.valid {
        return Err(
            "materials changed during preparation; run check --materials for details".into(),
        );
    }
    Ok(report)
}

// One lookup rule for check, fetch and build. Existing conflicting copies are
// errors, not permission to silently prefer the convenient one. Return the selected
// snapshot with its path; one inventory pass must not hash it again.
fn locate(
    root: &Path,
    cache: Option<&Path>,
    name: &str,
    digest: Option<&str>,
    remote: bool,
) -> Result<(std::path::PathBuf, Option<Content>), Failure> {
    if cache.is_none() || !remote {
        return Ok((root.join(name), None));
    }
    let mut paths = vec![root.join(name)];
    if remote && let Some(cache) = cache {
        paths.push(cache.join(name));
        if let Some(hash) = digest {
            paths.push(cache.join(".objects").join(hash.to_ascii_lowercase()));
        }
    }
    paths.dedup();
    let mut found: Option<(std::path::PathBuf, Content)> = None;
    for path in paths {
        match file_digest::read(&path) {
            Ok(content) => {
                if let Some((previous, expected)) = &found {
                    if *expected != content {
                        return Err(failure(
                            "material-conflict",
                            format!(
                                "conflicting material copies: {} and {} contain different bytes",
                                previous.display(),
                                path.display()
                            ),
                        ));
                    }
                } else {
                    found = Some((path, content));
                }
            }
            Err(file_digest::Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(failure("material-read", error)),
        }
    }
    Ok(found.map_or_else(
        || (root.join(name), None),
        |(path, content)| (path, Some(content)),
    ))
}

fn check_material(
    root: &Path,
    cache: Option<&Path>,
    collisions: &BTreeMap<String, String>,
    identity: String,
    material: spec::sources::Source,
) -> Record {
    let mut local = None;
    let checked = (|| {
        let (value, name, declared) = material_name(&material)?;
        local = Some(root.join(name));
        if let Some(message) = collisions.get(name) {
            return Err(failure("name-collision", message));
        }
        let (path, content) = locate(
            root,
            cache,
            name,
            declared.map(String::as_str),
            source::is_remote_url(value),
        )?;
        let path = local.insert(path);
        content
            .map_or_else(|| file_digest::read(path), Ok)
            .map_err(|error| {
                let code = match &error {
                    file_digest::Error::Io(error) if error.kind() == io::ErrorKind::NotFound => {
                        return failure(
                            "missing-file",
                            format!("{}: material file not found", path.display()),
                        );
                    }
                    file_digest::Error::Io(_) => "io-error",
                    file_digest::Error::NotRegular(_) => "not-regular-file",
                    file_digest::Error::Changed(_) => "material-changed",
                };
                failure(code, error)
            })
    })();
    let declared_sha256 = material.digest.ok().flatten();
    let outcome = match checked {
        Ok(content)
            if declared_sha256
                .as_ref()
                .is_none_or(|hash| hash.eq_ignore_ascii_case(&content.sha256)) =>
        {
            Outcome::Ready { content }
        }
        Ok(content) => Outcome::Error {
            content: Some(content),
            error: failure(
                "digest-mismatch",
                "staged bytes do not match the declared SHA-256; neither was changed",
            ),
        },
        Err(error) => Outcome::Error {
            content: None,
            error,
        },
    };
    Record {
        identity,
        expression: material.expression,
        resolved: material.url.ok(),
        path: local.map(|path| path.to_string_lossy().into_owned()),
        declared_sha256,
        outcome,
    }
}

impl Report {
    pub(crate) fn paths(&self) -> impl Iterator<Item = (&Path, &str)> {
        self.files.iter().filter_map(|row| {
            let path = Path::new(row.path.as_deref()?);
            let name = filename(row.resolved.as_deref()?).ok()?;
            Some((path, name))
        })
    }

    pub(crate) fn invalidate(&mut self, code: &'static str, message: impl std::fmt::Display) {
        self.valid = false;
        self.error = Some(failure(code, message));
    }

    pub(crate) fn write_human(&self, stdout: &mut impl Write) -> io::Result<()> {
        for row in &self.files {
            let detail = match &row.outcome {
                Outcome::Ready { content } => {
                    format!("{} bytes sha256={}", content.size, content.sha256)
                }
                Outcome::Error { error, .. } => format!("{}: {}", error.code, error.message),
            };
            writeln!(
                stdout,
                "{} {}: {detail}",
                row.identity,
                row.path.as_deref().unwrap_or("(unresolved)")
            )?;
        }
        if let Some(error) = &self.error {
            writeln!(stdout, "error[{}]: {}", error.code, error.message)?;
        }
        writeln!(
            stdout,
            "Materials: {}. Patch application and build not checked.",
            if self.valid { "PASS" } else { "FAIL" }
        )
    }
}

impl Report {
    /// Validated OBS/build filenames may differ from content-addressed cache paths.
    pub(crate) fn delivery_files(&self) -> Result<Vec<(String, std::path::PathBuf)>, String> {
        if !self.valid {
            return Err("material inventory is incomplete".into());
        }
        self.files
            .iter()
            .map(|record| {
                let name = filename(record.resolved.as_deref().ok_or("unresolved material")?)
                    .map_err(|e| e.message)?;
                Ok((
                    name.to_owned(),
                    std::path::PathBuf::from(
                        record.path.as_deref().ok_or("material has no local path")?,
                    ),
                ))
            })
            .collect()
    }
}
