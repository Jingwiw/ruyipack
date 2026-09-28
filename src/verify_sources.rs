// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Compare downloaded bytes with declarations, never replace the declarations.

use crate::{
    output_cli::{self, ReportFormat},
    render::manifest,
    source, spec, utf8_file,
};
use clap::Args;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::PathBuf,
};

#[derive(Args)]
pub(crate) struct Options {
    /// SPEC whose Source expressions can be resolved statically.
    #[arg(
        value_name = "SPEC",
        required_unless_present = "manifest",
        conflicts_with = "manifest"
    )]
    spec: Option<PathBuf>,
    /// Verify a handwritten manifest instead.
    #[arg(long, value_name = "PATH")]
    manifest: Option<PathBuf>,
    /// Define a static macro before reading the SPEC, in order.
    #[arg(
        short = 'D',
        long = "define",
        value_name = "MACRO EXPR",
        conflicts_with = "manifest"
    )]
    defines: Vec<String>,
    /// Print per-Source comparisons or a JSON report. No output writes back to the input.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(Serialize)]
struct Comparison {
    expression: String,
    declared_sha256: Option<String>,
    #[serde(flatten)]
    outcome: Outcome,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
enum Outcome {
    Match {
        download: source::Download,
    },
    Mismatch {
        download: source::Download,
    },
    Missing {
        download: source::Download,
    },
    Error {
        #[serde(flatten)]
        error: source::Error,
    },
    Unresolved {
        #[serde(flatten)]
        error: source::Error,
    },
    NotApplicable {
        reason: &'static str,
    },
}

impl Comparison {
    fn compare(
        expression: String,
        url: Result<String, String>,
        digest: Result<Option<String>, String>,
    ) -> Self {
        let declared_sha256 = digest.as_ref().ok().cloned().flatten();
        let outcome = match url {
            Err(message) => Outcome::Unresolved {
                error: source::Error::resolution(message),
            },
            Ok(url) if !url.contains(':') => Outcome::NotApplicable {
                reason: "local material; remote verification only",
            },
            Ok(url) => {
                let download = (|| {
                    let digest = digest.map_err(source::Error::invalid_digest)?;
                    if let Some(hash) = &digest {
                        source::validate_sha256(hash).map_err(source::Error::invalid_digest)?;
                    }
                    source::RemoteSource::parse(&url)?.download()
                })();
                match download {
                    Ok(download) => match &declared_sha256 {
                        None => Outcome::Missing { download },
                        Some(hash) if hash.eq_ignore_ascii_case(&download.sha256) => {
                            Outcome::Match { download }
                        }
                        Some(_) => Outcome::Mismatch { download },
                    },
                    Err(error) => Outcome::Error { error },
                }
            }
        };
        Self {
            expression,
            declared_sha256,
            outcome,
        }
    }
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let input = options
        .manifest
        .as_ref()
        .or(options.spec.as_ref())
        .expect("required CLI input");
    let mut input_sha256 = None;
    let mut sources = BTreeMap::new();
    let mut incomplete = None;
    let result = (|| {
        let path = fs::canonicalize(input)
            .map_err(|e| ("input-read", format!("{}: {e}", input.display())))?;
        let original = utf8_file::read(&path).map_err(|e| ("input-read", e.to_string()))?;
        input_sha256 = Some(utf8_file::sha256(&original));
        if options.manifest.is_some() {
            let manifest =
                manifest::parse(&original).map_err(|e| ("invalid-manifest", e.to_string()))?;
            let package = &manifest.package;
            for (number, material) in &manifest.sources {
                let comparison = match material {
                    manifest::Source::Local { path } => {
                        Comparison::compare(path.clone(), Ok(path.clone()), Ok(None))
                    }
                    manifest::Source::Remote { url, sha256 } => {
                        let resolved = spec::expression::substitute_fields(
                            url,
                            &[
                                ("name", &package.name),
                                ("version", &package.version),
                                ("url", &package.url),
                            ],
                        );
                        Comparison::compare(url.clone(), resolved, Ok(sha256.clone()))
                    }
                };
                sources.insert(*number, comparison);
            }
        } else {
            let parsed = spec::ParsedSpec::parse(&original);
            let resolved = spec::sources::resolve(&parsed, &options.defines)
                .map_err(|e| ("source-resolution", e))?;
            incomplete = resolved.incomplete;
            for (number, source) in resolved.sources {
                sources.insert(
                    number,
                    Comparison::compare(source.expression, source.url, source.digest),
                );
            }
        }
        if !utf8_file::is_unchanged(&path, &original).map_err(|e| ("input-read", e.to_string()))? {
            return Err(("source-changed", "input changed during verification; results describe the recorded input, not the current file".into()));
        }
        if let Some(reason) = incomplete {
            return Err(("source-resolution", reason));
        }
        Ok::<_, (&str, String)>(())
    })();
    let valid = result.is_ok()
        && sources.values().all(|item| {
            matches!(
                item.outcome,
                Outcome::Match { .. } | Outcome::NotApplicable { .. }
            )
        });
    let mut stdout = io::stdout().lock();
    if matches!(options.format, ReportFormat::Json) {
        let report = serde_json::json!({
            "tool": crate::tool::identity(),
            "format_version": 1, "scope": "remote-source-content", "valid": valid,
            "input": {"display_path": input.to_string_lossy(), "sha256": input_sha256},
            "defines": options.defines, "sources": sources,
            "error": result.as_ref().err().map(|(code, message)| if *code == "source-resolution" {
                source::Error::resolution(message).report(code)
            } else { output_cli::failure(code, message) }),
        });
        serde_json::to_writer(&mut stdout, &report).map_err(|e| e.to_string())?;
        writeln!(stdout).map_err(|e| e.to_string())?;
    } else {
        for (number, item) in &sources {
            let detail = match &item.outcome {
                Outcome::Match { download } => format!("match {}", download.sha256),
                Outcome::Mismatch { download } => format!(
                    "mismatch declared={} downloaded={}",
                    item.declared_sha256.as_deref().unwrap_or(""),
                    download.sha256
                ),
                Outcome::Missing { download } => {
                    format!("missing declared SHA-256; downloaded={}", download.sha256)
                }
                Outcome::Unresolved { error } => format!("unresolved: {error}"),
                Outcome::Error { error } => format!("error: {error}"),
                Outcome::NotApplicable { reason } => format!("not-applicable: {reason}"),
            };
            writeln!(stdout, "Source{number}: {detail}").map_err(|e| e.to_string())?;
        }
        result.map_err(|(_, message)| message)?;
        writeln!(stdout, "Input unchanged; no digests written. Matching bytes do not prove upstream authenticity.").map_err(|e| e.to_string())?;
    }
    Ok(valid)
}
