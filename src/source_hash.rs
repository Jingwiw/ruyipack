// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! CLI presentation for explicitly requested Source hashing.

use crate::{output_cli::ReportFormat, source, workspace::SpecOptions};
use clap::Args;
use serde::Serialize;
use std::io::{self, Write};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("hash-input").args(["work", "spec"]).required(true)))]
pub(crate) struct Options {
    #[command(flatten)]
    input: SpecOptions,
    /// Effective RPM Source number, not its position in the file.
    #[arg(long = "source", default_value_t = 0, value_name = "SOURCE")]
    source_number: u32,
    /// Define a static macro before reading the SPEC, in order.
    #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
    defines: Vec<String>,
    /// Print a digest and next steps, or a TOML success/failure report.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let mut input = None;
    let result = (|| {
        input = Some(
            options
                .input
                .resolve()
                .map_err(|e| HashError::Input(e.to_string()))?,
        );
        let input = input.as_ref().expect("resolved input");
        let parsed = crate::spec::ParsedSpec::parse(&input.source);
        let resolved = crate::spec::sources::resolve(&parsed, &options.defines)
            .map_err(|error| HashError::Source(source::Error::resolution(error)))?;
        let result = source::download_selected(&resolved, &[options.source_number])
            .map_err(HashError::Source)?;
        if !input
            .is_unchanged()
            .map_err(|e| HashError::Input(e.to_string()))?
        {
            return Err(HashError::Changed);
        }
        Ok::<_, HashError>(result)
    })();
    let input_sha256 = input
        .as_ref()
        .map(|input| crate::utf8_file::sha256(&input.source));
    let valid = result.is_ok();
    let mut stdout = io::stdout().lock();
    if matches!(options.format, ReportFormat::Toml) {
        let display = options.input.display();
        let report = HashReport {
            format_version: 2,
            tool: crate::tool::identity(),
            valid,
            source: options.source_number,
            input: crate::report::Input {
                display_path: input
                    .as_ref()
                    .map_or_else(|| display.into(), |input| input.path.to_string_lossy()),
                sha256: input_sha256.as_deref(),
                revision: input.as_ref().and_then(|input| input.revision.as_deref()),
            },
            defines: &options.defines,
            download: result
                .as_ref()
                .ok()
                .map(|value| &value[&options.source_number]),
            error: result.as_ref().err().map(|error| match error {
                HashError::Source(error) => error.report("source-hash-failed"),
                HashError::Input(_) => {
                    source::Failure::General(crate::report::failure("input-read", error))
                }
                HashError::Changed => {
                    source::Failure::General(crate::report::failure("source-changed", error))
                }
            }),
        };
        crate::report::write(&mut stdout, &report).map_err(|e| e.to_string())?;
    } else {
        let value = result.map_err(|error| error.to_string())?;
        let input = input.as_ref().expect("successful input");
        let digest = &value[&options.source_number].sha256;
        let input_sha256 = input_sha256.as_deref().expect("successful input");
        writeln!(stdout, "{digest}").map_err(|e| e.to_string())?;
        let target = if let Some(work) = &options.input.work {
            shell_words::quote(work).into_owned()
        } else {
            format!(
                "--spec={}",
                shell_words::quote(&input.path.to_string_lossy())
            )
        };
        let assignment = format!("sources.{}.sha256={digest}", options.source_number);
        writeln!(io::stderr().lock(),
            "SHA-256 calculated; SPEC unchanged. For an adjacent RemoteAsset marker:\nPreview: ruyipack edit --expect-sha256 {} --set {assignment} --diff {target}\nApply: ruyipack edit --apply --expect-sha256 {} --set {assignment} {target}", input_sha256, input_sha256)
            .map_err(|e| e.to_string())?;
    }
    Ok(valid)
}

#[derive(Debug, thiserror::Error)]
enum HashError {
    #[error("{0}")]
    Input(String),
    #[error(transparent)]
    Source(source::Error),
    #[error("SPEC changed during source hashing; rerun against the new input")]
    Changed,
}

#[derive(Serialize)]
struct HashReport<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    valid: bool,
    source: u32,
    input: crate::report::Input<'a>,
    defines: &'a [String],
    #[serde(flatten)]
    download: Option<&'a source::Download>,
    error: Option<source::Failure<'a>>,
}
