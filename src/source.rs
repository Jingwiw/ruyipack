// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source URL policy and explicit downloads, shared by generation and editing.
//! Native resolution is opt-in; static expression checks never execute RPM macros.

use crate::{spec, utf8_file};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    fs,
    io::{self, BufRead},
    path::Path,
    process::Command,
};
use url::{SyntaxViolation, Url};

/// Checks a Source using only already-known package fields, preserving its spelling.
/// RPM syntax is recognized before URL validation, including escaped literal percent signs.
pub(crate) fn validate_expression(value: &str, fields: &[(&str, &str)]) -> Result<Url, String> {
    let resolved = crate::spec::expression::substitute_fields(value, fields)?;
    authoring_url(&resolved)
}

/// Checks URL syntax independently of the generator's HTTPS-only publishing policy.
pub(crate) fn validate_url(value: &str) -> Result<Url, String> {
    let invalid = || {
        "expected an absolute HTTP or HTTPS URL without whitespace or repaired syntax".to_owned()
    };
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    let repaired = Cell::new(false);
    let capture = |violation| {
        if matches!(
            violation,
            SyntaxViolation::ExpectedDoubleSlash
                | SyntaxViolation::Backslash
                | SyntaxViolation::NonUrlCodePoint
                | SyntaxViolation::PercentDecode
        ) {
            repaired.set(true);
        }
    };
    let parsed = Url::options()
        .syntax_violation_callback(Some(&capture))
        .parse(value)
        .map_err(|_| invalid())?;
    if repaired.get() || parsed.host().is_none() {
        return Err(invalid());
    }
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(invalid());
    }
    Ok(parsed)
}

/// Parse once, then apply authoring policy without exposing credentials in errors.
pub(crate) fn authoring_url(value: &str) -> Result<Url, String> {
    let url = validate_url(value)?;
    credentials(&url)?;
    Ok(url)
}

/// Existing package URLs may contain unresolved macros; syntax checks own those results.
pub(crate) fn reject_credentials(value: &str) -> Result<(), String> {
    Url::parse(value).map_or(Ok(()), |url| credentials(&url))
}

fn credentials(url: &Url) -> Result<(), String> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "URL credentials are not allowed; supply authentication outside the SPEC".into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_sha256(value: &str) -> Result<(), &'static str> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("expected 64 hexadecimal digits")
    }
}

/// New manifests require HTTPS; existing SPEC editing also accepts HTTP.
pub(crate) fn require_https(field: &str, url: Url) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err(format!("{field}: expected an HTTPS URL"));
    }
    Ok(())
}

#[derive(Serialize)]
pub(crate) struct Download {
    resolved_url: String,
    effective_url: String,
    pub(crate) sha256: String,
    bytes: u64,
}

#[derive(Serialize)]
pub(crate) struct NativeEvidence {
    rpm: String,
    defines: Vec<String>,
    working_directory: String,
    expanded_spec_sha256: String,
    temporary_candidate: bool,
}

#[derive(Serialize)]
pub(crate) struct SourceHashes {
    pub(crate) input_sha256: String,
    pub(crate) native: NativeEvidence,
    pub(crate) sources: std::collections::BTreeMap<u32, Download>,
}

/// Resolve all requested Sources once, then hash their actual downloaded bytes.
/// Pending edits are parsed from a private, same-named file with the original cwd.
pub(crate) fn calculate(
    path: &Path,
    contents: &str,
    numbers: &[u32],
    defines: &[String],
) -> Result<SourceHashes, String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let directory = path.parent().ok_or("SPEC has no parent directory")?;
    let temporary = (utf8_file::read(&path).map_err(|e| e.to_string())? != contents)
        .then(tempfile::tempdir)
        .transpose()
        .map_err(|e| e.to_string())?;
    let native_path = if let Some(temp) = &temporary {
        // A pathname-dependent Source cannot be resolved faithfully on a private copy.
        if contents.contains("__file_name") || defines.iter().any(|d| d.contains("__file_name")) {
            return Err(
                "native candidate uses __file_name; save a reviewed copy before hashing it".into(),
            );
        }
        let candidate = temp
            .path()
            .join(path.file_name().ok_or("SPEC has no filename")?);
        fs::write(&candidate, contents).map_err(|e| e.to_string())?;
        candidate
    } else {
        path.clone()
    };
    let rpm_version = checked(Command::new("rpmspec").arg("--version"), "rpmspec")?;
    let mut rpm = Command::new("rpmspec");
    rpm.current_dir(directory).env("LC_ALL", "C").arg("--parse");
    for define in defines {
        rpm.arg("--define").arg(define);
    }
    rpm.arg(native_path);
    let expanded = checked(
        &mut rpm,
        "native RPM resolution (use target distribution macro packages and repository configuration; pass project-specific macros with --define)",
    )?;
    let mut urls = std::collections::BTreeMap::new();
    // Resolve every URL before downloading, so an unsupported selection has no network side effects.
    for number in numbers {
        let url = spec::native::source_url(&expanded, *number)?;
        let remote = RemoteSource::parse(url).map_err(|e| {
            format!("Source{number}: {e}; unresolved macros or local Sources cannot be downloaded")
        })?;
        urls.insert(*number, remote);
    }
    let sources = urls
        .into_iter()
        .map(|(number, url)| Ok((number, url.download()?)))
        .collect::<Result<_, String>>()?;
    Ok(SourceHashes {
        input_sha256: utf8_file::digest(contents),
        native: NativeEvidence {
            rpm: rpm_version.trim().to_owned(),
            defines: defines.to_vec(),
            working_directory: directory.to_string_lossy().into_owned(),
            expanded_spec_sha256: utf8_file::digest(&expanded),
            temporary_candidate: temporary.is_some(),
        },
        sources,
    })
}

pub(crate) fn unchanged(path: &Path, original: &str) -> Result<(), String> {
    if !utf8_file::is_unchanged(path, original).map_err(|e| e.to_string())? {
        return Err("SPEC changed during source hashing; rerun against the new input".into());
    }
    Ok(())
}

/// A checked remote URL retains its original spelling for evidence.
/// Native batches prepare every Source before any download is allowed to start.
pub(crate) struct RemoteSource<'url> {
    original: &'url str,
    url: Url,
}

impl<'url> RemoteSource<'url> {
    pub(crate) fn parse(original: &'url str) -> Result<Self, String> {
        Ok(Self {
            original,
            url: authoring_url(original)?,
        })
    }

    /// Hash the response bytes without RPM execution.
    pub(crate) fn download(mut self) -> Result<Download, String> {
        // RPM's #/filename suffix names a local archive; it is not part of the HTTP request.
        self.url.set_fragment(None);
        let asset = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        // Like openRuyi's remoteassetify.py, reuse curl rather than another HTTP/TLS stack.
        // Disable curlrc and constrain redirects too; never trust a partially downloaded file.
        let effective_url = checked(
            Command::new("curl")
                .args([
                    "--disable",
                    "--fail",
                    "--location",
                    "--silent",
                    "--show-error",
                    "--proto",
                    "=http,https",
                    "--proto-redir",
                    "=http,https",
                    "--output",
                ])
                .arg(asset.path())
                .args(["--write-out", "%{url_effective}", "--", self.url.as_str()]),
            "curl download",
        )?;
        authoring_url(&effective_url)?;
        let mut reader = io::BufReader::new(asset.as_file());
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        loop {
            let buffer = reader.fill_buf().map_err(|e| e.to_string())?;
            if buffer.is_empty() {
                break;
            }
            digest.update(buffer);
            let length = buffer.len();
            bytes += length as u64;
            reader.consume(length);
        }
        Ok(Download {
            resolved_url: self.original.to_owned(),
            effective_url,
            sha256: format!("{:x}", digest.finalize()),
            bytes,
        })
    }
}

fn checked(command: &mut Command, stage: &str) -> Result<String, String> {
    let output = command.output().map_err(|e| format!("{stage}: {e}"))?;
    // RPM can exit zero while printing an error. Warnings also leave native
    // completeness unproven; do not quietly turn them into an accepted digest.
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(format!(
            "{stage}: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("{stage}: {e}"))
}
