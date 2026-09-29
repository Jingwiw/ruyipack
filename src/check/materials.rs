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

struct Declaration {
    identity: String,
    expression: String,
    value: Result<String, String>,
    digest: Result<Option<String>, String>,
}

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
            return Err(failure("source-directory", "expected a directory"));
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
        let materials = declarations
            .into_iter()
            .map(|(identity, s)| Declaration {
                identity,
                expression: s.expression,
                value: s.url,
                digest: s.digest,
            })
            .collect::<Vec<_>>();
        let incomplete = resolved.incomplete;
        let mut names: BTreeMap<&str, Vec<&Declaration>> = BTreeMap::new();
        for material in &materials {
            if let Ok(value) = &material.value
                && let Ok(name) = filename(value)
            {
                names.entry(name).or_default().push(material);
            }
        }
        for material in &materials {
            let mut local = None;
            let mut observed = None;
            let checked = (|| {
                let value = material
                    .value
                    .as_ref()
                    .map_err(|e| failure("unresolved", e))?;
                let name = filename(value)?;
                local = Some(root.join(name));
                let peers = &names[name];
                // Identical declarations may intentionally reuse one file. Different
                // declared locations targeting that file are ambiguous, even if bytes match.
                if peers
                    .iter()
                    .any(|peer| peer.value.as_ref().ok() != Some(value))
                {
                    return Err(failure(
                        "name-collision",
                        format!(
                            "{name} is targeted by {}",
                            peers
                                .iter()
                                .map(|p| p.identity.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                }
                let declared = material
                    .digest
                    .as_ref()
                    .map_err(|e| failure("invalid-digest", e))?;
                if let Some(hash) = declared {
                    source::validate_sha256(hash).map_err(|e| failure("invalid-digest", e))?;
                }
                let bytes = content(local.as_deref().expect("resolved path"))?;
                let matches = declared
                    .as_ref()
                    .is_none_or(|hash| hash.eq_ignore_ascii_case(&bytes.sha256));
                observed = Some(bytes);
                if !matches {
                    return Err(failure(
                        "digest-mismatch",
                        "staged bytes do not match the declared SHA-256; neither was changed",
                    ));
                }
                Ok(())
            })();
            let outcome = match checked {
                Ok(()) => Outcome::Ready {
                    content: observed.expect("successful file read"),
                },
                Err(error) => Outcome::Error {
                    content: observed,
                    error,
                },
            };
            records.push(Record {
                identity: material.identity.clone(),
                expression: material.expression.clone(),
                resolved: material.value.as_ref().ok().cloned(),
                path: local,
                declared_sha256: material.digest.as_ref().ok().cloned().flatten(),
                outcome,
            });
        }
        if !utf8_file::is_unchanged(&path, original).map_err(|e| failure("input-read", e))? {
            return Err(failure(
                "input-changed",
                "recipe changed during inventory; retry",
            ));
        }
        if let Some(reason) = incomplete {
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
