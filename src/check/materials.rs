// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only inventory of a prepared RPM source directory, not a build admission policy.

use crate::{source, spec, utf8_file};
use fs_err as fs;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize)]
struct Failure {
    code: &'static str,
    message: String,
}
fn failure(code: &'static str, message: impl ToString) -> Failure {
    Failure {
        code,
        message: message.to_string(),
    }
}

#[derive(Serialize)]
struct Content {
    size: u64,
    sha256: String,
}

#[derive(Serialize)]
struct Record {
    identity: String,
    expression: String,
    resolved: Option<String>,
    path: Option<PathBuf>,
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

fn content(path: &Path) -> Result<Content, Failure> {
    let io_error = |e: io::Error| {
        failure(
            if e.kind() == io::ErrorKind::NotFound {
                "missing-file"
            } else {
                "io-error"
            },
            e,
        )
    };
    let before = fs::symlink_metadata(path).map_err(io_error)?;
    // Reject symlinks and special files before opening: preflight does not follow
    // external material links or block waiting for a FIFO. This is not a hostile-directory sandbox.
    if !before.is_file() {
        return Err(failure(
            "not-regular-file",
            format!(
                "{}: stage a regular file, not a symlink or special file",
                path.display()
            ),
        ));
    }
    let mut file = fs::File::open(path)
        .map_err(io_error)?
        .take(before.len().saturating_add(1));
    let mut digest = Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        size += read as u64;
    }
    let after = fs::symlink_metadata(path).map_err(io_error)?;
    if !after.is_file()
        || size != before.len()
        || after.len() != before.len()
        || after.modified().map_err(io_error)? != before.modified().map_err(io_error)?
    {
        return Err(failure(
            "material-changed",
            format!(
                "{} changed while hashing; retry against stable files",
                path.display()
            ),
        ));
    }
    Ok(Content {
        size,
        sha256: format!("{:x}", digest.finalize()),
    })
}

#[derive(Serialize)]
pub(crate) struct Report {
    pub(crate) valid: bool,
    source_dir: Option<PathBuf>,
    files: Vec<Record>,
    error: Option<Failure>,
}

/// Checks only declared inputs in a stable, prepared _sourcedir. No fetching or writes.
pub(crate) fn analyze(
    input: &Path,
    original: &str,
    parsed: &spec::ParsedSpec<'_>,
    defines: &[String],
    source_dir: Option<&Path>,
) -> Report {
    let mut directory = None;
    let mut records = Vec::new();
    let result = (|| {
        let path = fs::canonicalize(input).map_err(|e| failure("input-read", e))?;
        let root = fs::canonicalize(source_dir.unwrap_or(path.parent().expect("absolute input")))
            .map_err(|e| failure("source-directory", e))?;
        if !root.is_dir() {
            return Err(failure(
                "source-directory",
                format!("{}: expected a directory", root.display()),
            ));
        }
        directory = Some(root.clone());
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
        for (identity, material) in declarations {
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
                content(path)
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
            records.push(Record {
                identity,
                expression: material.expression,
                resolved: material.url.ok(),
                path: local,
                declared_sha256,
                outcome,
            });
        }
        if !utf8_file::is_unchanged(&path, original).map_err(|e| failure("input-read", e))? {
            return Err(failure(
                "input-changed",
                "recipe changed during inventory; retry",
            ));
        }
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

impl Report {
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
                row.path
                    .as_deref()
                    .map_or_else(|| "(unresolved)".into(), |p| p.display().to_string())
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
