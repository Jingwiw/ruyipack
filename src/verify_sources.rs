// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Compare downloaded bytes with declarations, never replace the declarations.

use crate::{
    output_cli::ReportFormat, render::manifest, source, spec, utf8_file, workspace::SpecOptions,
};
use clap::Args;
use fs_err as fs;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::PathBuf,
};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("verify-input").args(["work", "spec", "manifest"]).required(true)))]
pub(crate) struct Options {
    #[command(flatten)]
    input: SpecOptions,
    /// Verify a handwritten manifest instead.
    #[arg(long, value_name = "PATH", required_unless_present_any = ["work", "spec"],
        conflicts_with_all = ["work", "spec", "pkgname"])]
    manifest: Option<PathBuf>,
    /// Define a static macro before reading the SPEC, in order.
    #[arg(
        short = 'D',
        long = "define",
        value_name = "MACRO EXPR",
        conflicts_with = "manifest"
    )]
    defines: Vec<String>,
    /// Print per-Source comparisons or a TOML report. No output writes back to the input.
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
        let outcome = match url {
            Err(message) => Outcome::Unresolved {
                error: source::Error::resolution(message),
            },
            Ok(url) if !url.contains(':') => Outcome::NotApplicable {
                reason: "local material; remote verification only",
            },
            Ok(url) => {
                let download = (|| {
                    let digest = digest.as_ref().map_err(source::Error::invalid_digest)?;
                    if let Some(hash) = digest {
                        source::validate_sha256(hash).map_err(source::Error::invalid_digest)?;
                    }
                    source::RemoteSource::parse(&url)?.download()
                })();
                match download {
                    Ok(download) => match digest.as_ref().ok().and_then(|hash| hash.as_ref()) {
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
            declared_sha256: digest.ok().flatten(),
            outcome,
        }
    }
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let mut display_path = options.manifest.as_ref().map_or_else(
        || options.input.display(),
        |path| path.to_string_lossy().into_owned(),
    );
    let mut revision = None;
    let mut input_sha256 = None;
    let mut sources = BTreeMap::new();
    let mut incomplete = None;
    let result = (|| {
        let manifest_input = options
            .manifest
            .as_ref()
            .map(|path| {
                let path = fs::canonicalize(path).map_err(|e| ("input-read", e.to_string()))?;
                let source = utf8_file::read(&path).map_err(|e| ("input-read", e.to_string()))?;
                Ok::<_, (&str, String)>((path, source))
            })
            .transpose()?;
        let spec_input = if manifest_input.is_none() {
            Some(
                options
                    .input
                    .resolve()
                    .map_err(|e| ("input-read", e.to_string()))?,
            )
        } else {
            None
        };
        let original = if let Some((path, source)) = &manifest_input {
            display_path = path.to_string_lossy().into_owned();
            source
        } else {
            let input = spec_input.as_ref().expect("SPEC or manifest input");
            display_path = input.path.to_string_lossy().into_owned();
            revision.clone_from(&input.revision);
            &input.source
        };
        input_sha256 = Some(utf8_file::sha256(original));
        if options.manifest.is_some() {
            let manifest =
                manifest::parse(original).map_err(|e| ("invalid-manifest", e.to_string()))?;
            let package = &manifest.package;
            for (number, material) in manifest.sources {
                let comparison = match material {
                    manifest::Source::Local { path } => Comparison {
                        expression: path,
                        declared_sha256: None,
                        outcome: Outcome::NotApplicable {
                            reason: "local material; remote verification only",
                        },
                    },
                    manifest::Source::Remote { url, sha256 } => {
                        let resolved = manifest::resolve_source(
                            &url,
                            &package.name,
                            &package.version,
                            &package.url,
                        );
                        Comparison::compare(url, resolved, Ok(sha256))
                    }
                };
                sources.insert(number, comparison);
            }
        } else {
            let parsed = spec::ParsedSpec::parse(original);
            let resolved = spec::sources::resolve(&parsed, &options.defines)
                .map_err(|e| ("source-resolution", e))?;
            incomplete.clone_from(&resolved.incomplete);
            for (&number, source) in &resolved.sources {
                sources.insert(
                    number,
                    Comparison::compare(
                        source.expression.clone(),
                        source.url.clone(),
                        source.digest.clone(),
                    ),
                );
            }
        }
        let unchanged = if let Some((path, source)) = &manifest_input {
            utf8_file::is_unchanged(path, source)
        } else {
            spec_input.as_ref().expect("SPEC input").is_unchanged()
        }
        .map_err(|e| ("input-read", e.to_string()))?;
        if !unchanged {
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
    if matches!(options.format, ReportFormat::Toml) {
        let resolution_error = result
            .as_ref()
            .err()
            .filter(|(code, _)| *code == "source-resolution")
            .map(|(_, message)| source::Error::resolution(message));
        let error = result
            .as_ref()
            .err()
            .map(|(code, message)| match &resolution_error {
                Some(error) => error.report(code),
                None => source::Failure::General(crate::report::failure(code, message)),
            });
        let report = VerifyReport {
            tool: crate::tool::identity(),
            format_version: 1,
            scope: "remote-source-content",
            valid,
            input: crate::report::Input {
                display_path: display_path.as_str().into(),
                sha256: input_sha256.as_deref(),
                revision: revision.as_deref(),
            },
            defines: &options.defines,
            sources: crate::report::Numbered(&sources),
            error,
        };
        crate::report::write(&mut stdout, &report).map_err(|e| e.to_string())?;
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

#[derive(Serialize)]
struct VerifyReport<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    scope: &'static str,
    valid: bool,
    input: crate::report::Input<'a>,
    defines: &'a [String],
    sources: crate::report::Numbered<'a, Comparison>,
    error: Option<source::Failure<'a>>,
}
