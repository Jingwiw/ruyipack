// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only inventory of a prepared RPM source directory, not a build admission policy.

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

// RPM 6 newSource(): last slash, then last '=' in that suffix, with no URL decoding.
// This is the build filename, not a path to copy from or a downloader's URL parser.
fn filename(value: &str) -> Result<&str, Failure> {
    let name = value.rsplit_once('/').map_or(value, |(_, tail)| {
        tail.rsplit_once('=').map_or(tail, |(_, name)| name)
    });
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(failure(
            "invalid-filename",
            "RPM material filename is empty, a directory, or contains whitespace/control characters",
        ));
    }
    Ok(name)
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
    parsed: &spec::ParsedSpec<'_>,
    defines: &[String],
) -> Report {
    let mut directory = None;
    let mut records = Vec::new();
    let result = (|| {
        let source_dir = source_dir.ok_or_else(|| failure(
            "source-directory",
            "a committed main SPEC has no checkout; provide --source-dir with its prepared materials",
        ))?;
        let root = fs::canonicalize(source_dir).map_err(|e| failure("source-directory", e))?;
        if !root.is_dir() {
            return Err(failure(
                "source-directory",
                format!("{}: expected a directory", root.display()),
            ));
        }
        directory = Some(root.to_string_lossy().into_owned());
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
        records.extend(
            declarations
                .into_iter()
                .map(|(identity, material)| check_material(&root, &collisions, identity, material)),
        );
        if let Some(reason) = resolved.incomplete {
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

fn check_material(
    root: &Path,
    collisions: &BTreeMap<String, String>,
    identity: String,
    material: spec::sources::Source,
) -> Record {
    let mut local = None;
    let checked = (|| {
        let value = material
            .url
            .as_ref()
            .map_err(|e| failure("unresolved", e))?;
        let name = filename(value)?;
        let path = local.insert(root.join(name));
        if let Some(message) = collisions.get(name) {
            return Err(failure("name-collision", message));
        }
        let declared = material
            .digest
            .as_ref()
            .map_err(|e| failure("invalid-digest", e))?;
        if let Some(hash) = declared {
            source::validate_sha256(hash).map_err(|e| failure("invalid-digest", e))?;
        }
        file_digest::read(path).map_err(|error| {
            let code = match &error {
                file_digest::Error::Io(error) if error.kind() == io::ErrorKind::NotFound => {
                    "missing-file"
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
            "{}: local material snapshot only; no downloads, writes, patch application or build validation.",
            if self.valid { "PASS" } else { "FAIL" }
        )
    }
}
