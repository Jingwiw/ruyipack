// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Explicit Source downloads shared by generation and controlled editing.

use crate::{output_cli::ReportFormat, source, spec, utf8_file};
use clap::Args;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[derive(Args)]
pub(crate) struct Options {
    /// SPEC in a prepared RPM environment; native macros may execute code.
    spec: PathBuf,
    /// Acknowledge native macro execution. Isolate untrusted input.
    #[arg(long, required = true)]
    trusted_spec: bool,
    /// Effective RPM Source number, not its position in the file.
    #[arg(long, default_value_t = 0)]
    source: u32,
    /// Pass an RPM macro definition verbatim, in order.
    #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
    defines: Vec<String>,
    /// Print a digest and next steps, or a JSON success/failure report.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Serialize)]
pub(crate) struct Download {
    pub(crate) resolved_url: String,
    pub(crate) effective_url: String,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
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

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let result = (|| {
        let path = fs::canonicalize(&options.spec).map_err(|e| e.to_string())?;
        let original = utf8_file::read(&path).map_err(|e| e.to_string())?;
        let result = calculate(&path, &original, &[options.source], &options.defines)?;
        unchanged(&path, &original)?;
        Ok::<_, String>((result, path))
    })();
    let valid = result.is_ok();
    let mut stdout = io::stdout().lock();
    if matches!(options.format, ReportFormat::Json) {
        let mut report = serde_json::json!({
            "format_version": 1, "valid": valid, "source": options.source,
            "input": { "display_path": options.spec.to_string_lossy() },
        });
        match result {
            Ok((value, _)) => {
                report["input"]["sha256"] = value.input_sha256.into();
                for (key, value) in serde_json::to_value(&value.sources[&options.source])
                    .expect("serializable download")
                    .as_object()
                    .expect("download object")
                {
                    report[key] = value.clone();
                }
                report["native"] =
                    serde_json::to_value(value.native).expect("serializable evidence");
            }
            Err(error) => report["error"] = error.into(),
        }
        serde_json::to_writer(&mut stdout, &report).map_err(|e| e.to_string())?;
        writeln!(stdout).map_err(|e| e.to_string())?;
    } else {
        let (value, path) = result?;
        let digest = &value.sources[&options.source].sha256;
        writeln!(stdout, "{digest}").map_err(|e| e.to_string())?;
        let display = path.to_string_lossy();
        let path = shell_words::quote(&display);
        let assignment = format!("sources.{}.sha256={digest}", options.source);
        writeln!(io::stderr().lock(),
            "SHA-256 calculated; SPEC unchanged. For an adjacent RemoteAsset marker:\nPreview: ruyipack edit --expect-sha256 {} --set {assignment} --diff -- {path}\nApply: ruyipack edit --expect-sha256 {} --set {assignment} -- {path}", value.input_sha256, value.input_sha256)
            .map_err(|e| e.to_string())?;
    }
    Ok(valid)
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
    let rpm_version = String::from_utf8(rpm_version.stdout).map_err(|e| e.to_string())?;
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
    let expanded = String::from_utf8(expanded.stdout).map_err(|e| e.to_string())?;
    let mut urls = std::collections::BTreeMap::new();
    // Resolve every URL before downloading, so an unsupported selection has no network side effects.
    for number in numbers {
        let url = spec::native::source_url(&expanded, *number)?;
        source::reject_credentials(url)?;
        source::validate_url(url).map_err(|e| {
            format!("Source{number}: {e}; unresolved macros or local Sources cannot be downloaded")
        })?;
        urls.insert(*number, url);
    }
    let sources = urls
        .into_iter()
        .map(|(number, url)| Ok((number, download(url)?)))
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

/// Hashes the actual response bytes of an already resolved URL, without RPM execution.
pub(crate) fn download(resolved_url: &str) -> Result<Download, String> {
    source::reject_credentials(resolved_url)?;
    source::validate_url(resolved_url)?;
    let mut url = url::Url::parse(resolved_url).map_err(|e| e.to_string())?;
    // RPM's #/filename suffix names a local archive; it is not part of the HTTP request.
    url.set_fragment(None);
    let asset = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    // Like openRuyi's remoteassetify.py, reuse curl rather than another HTTP/TLS stack.
    // Disable curlrc and constrain redirects too; never trust a partially downloaded file.
    let download = checked(
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
            .args(["--write-out", "%{url_effective}", "--", url.as_str()]),
        "curl download",
    )?;
    let effective_url = String::from_utf8(download.stdout).map_err(|e| e.to_string())?;
    source::reject_credentials(&effective_url)?;
    source::validate_url(&effective_url)?;
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
        resolved_url: resolved_url.to_owned(),
        effective_url,
        sha256: format!("{:x}", digest.finalize()),
        bytes,
    })
}

fn checked(command: &mut Command, stage: &str) -> Result<Output, String> {
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
    Ok(output)
}
