// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Explicit native/network operation; static authoring commands remain offline.

use crate::{output_cli::ReportFormat, source, spec, utf8_file};
use clap::Args;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    process::{Command, Output},
};

#[derive(Args)]
pub(crate) struct Options {
    /// SPEC in a prepared RPM environment; native macros may execute code.
    spec: PathBuf,
    /// Acknowledge native macro execution. Use an isolated environment for untrusted input.
    #[arg(long, required = true)]
    trusted_spec: bool,
    /// Effective RPM Source number (not the position in the file).
    #[arg(long, default_value_t = 0)]
    source: u32,
    /// Pass an RPM macro definition verbatim, in order; normal RPM precedence applies.
    #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
    defines: Vec<String>,
    /// Human output is SHA-256 alone; JSON also records URL, input and native context.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

pub(crate) fn run(options: &Options) -> Result<(), String> {
    let original = utf8_file::read(&options.spec).map_err(|e| e.to_string())?;
    let path = fs::canonicalize(&options.spec).map_err(|e| e.to_string())?;
    let directory = path.parent().ok_or("SPEC has no parent directory")?;
    let rpm_version = checked(Command::new("rpmspec").arg("--version"), "rpmspec")?;
    let rpm_version = String::from_utf8(rpm_version.stdout).map_err(|e| e.to_string())?;
    // Keep the real filename and directory: RPM macros/includes can depend on them.
    let mut rpm = Command::new("rpmspec");
    rpm.current_dir(directory).env("LC_ALL", "C").arg("--parse");
    for define in &options.defines {
        rpm.arg("--define").arg(define);
    }
    rpm.arg(&path);
    let expanded = checked(
        &mut rpm,
        "native RPM resolution (check target macros and explicit --define inputs)",
    )?;
    let expanded = String::from_utf8(expanded.stdout).map_err(|e| e.to_string())?;
    let resolved_url = spec::native::source_url(&expanded, options.source)?;
    source::reject_credentials(resolved_url)?;
    source::validate_url(resolved_url).map_err(|e| {
        format!(
            "Source{}: {e}; unresolved macros or local Sources cannot be downloaded",
            options.source
        )
    })?;
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
    if utf8_file::read(&path).map_err(|e| e.to_string())? != original {
        return Err("SPEC changed during source hashing; rerun against the new input".into());
    }
    let sha256 = format!("{:x}", digest.finalize());
    let mut stdout = io::stdout().lock();
    if matches!(options.format, ReportFormat::Json) {
        let report = serde_json::json!({
            "format_version": 1,
            "input": { "display_path": options.spec.to_string_lossy(), "sha256": utf8_file::digest(&original) },
            "source": options.source,
            "resolved_url": resolved_url,
            "effective_url": effective_url,
            "sha256": sha256,
            "bytes": bytes,
            "native": { "rpm": rpm_version.trim(), "defines": options.defines, "working_directory": directory.to_string_lossy(), "expanded_spec_sha256": utf8_file::digest(&expanded) },
        });
        serde_json::to_writer(&mut stdout, &report).map_err(|e| e.to_string())?;
        writeln!(stdout).map_err(|e| e.to_string())
    } else {
        writeln!(stdout, "{sha256}").map_err(|e| e.to_string())
    }
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
