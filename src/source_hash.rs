// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! CLI presentation for explicitly requested Source hashing.

use crate::{
    output_cli::{self, ReportFormat},
    source, utf8_file,
};
use clap::Args;
use fs_err as fs;
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Args)]
pub(crate) struct Options {
    /// SPEC whose Source expressions can be resolved statically.
    spec: PathBuf,
    /// Effective RPM Source number, not its position in the file.
    #[arg(long = "source", default_value_t = 0, value_name = "SOURCE")]
    source_number: u32,
    /// Define a static macro before reading the SPEC, in order.
    #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
    defines: Vec<String>,
    /// Print a digest and next steps, or a JSON success/failure report.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let result = (|| {
        let path = fs::canonicalize(&options.spec).map_err(|e| HashError::Input(e.to_string()))?;
        let original = utf8_file::read(&path).map_err(|e| HashError::Input(e.to_string()))?;
        let result = source::calculate(&original, &[options.source_number], &options.defines)
            .map_err(HashError::Source)?;
        if !utf8_file::is_unchanged(&path, &original)
            .map_err(|e| HashError::Input(format!("{}: {e}", path.display())))?
        {
            return Err(HashError::Changed);
        }
        Ok::<_, HashError>((result, path))
    })();
    let valid = result.is_ok();
    let mut stdout = io::stdout().lock();
    if matches!(options.format, ReportFormat::Json) {
        let display_path = options.spec.to_string_lossy();
        let mut report = match result {
            Ok((value, _)) => {
                let mut report = serde_json::to_value(&value.sources[&options.source_number])
                    .expect("serializable download");
                report["input"] =
                    serde_json::json!({"display_path": display_path, "sha256": value.input_sha256});
                report["defines"] = serde_json::json!(value.defines);
                report
            }
            Err(error) => {
                let error = match &error {
                    HashError::Source(error) => error.report("source-hash-failed"),
                    HashError::Input(_) => output_cli::failure("input-read", error),
                    HashError::Changed => output_cli::failure("source-changed", error),
                };
                serde_json::json!({"input": {"display_path": display_path}, "error": error})
            }
        };
        report["format_version"] = 2.into();
        report["tool"] = serde_json::json!(crate::tool::identity());
        report["valid"] = valid.into();
        report["source"] = options.source_number.into();
        serde_json::to_writer(&mut stdout, &report).map_err(|e| e.to_string())?;
        writeln!(stdout).map_err(|e| e.to_string())?;
    } else {
        let (value, path) = result.map_err(|error| error.to_string())?;
        let digest = &value.sources[&options.source_number].sha256;
        writeln!(stdout, "{digest}").map_err(|e| e.to_string())?;
        let display = path.to_string_lossy();
        let path = shell_words::quote(&display);
        let assignment = format!("sources.{}.sha256={digest}", options.source_number);
        writeln!(io::stderr().lock(),
            "SHA-256 calculated; SPEC unchanged. For an adjacent RemoteAsset marker:\nPreview: ruyipack edit --expect-sha256 {} --set {assignment} --diff -- {path}\nApply: ruyipack edit --expect-sha256 {} --set {assignment} -- {path}", value.input_sha256, value.input_sha256)
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
