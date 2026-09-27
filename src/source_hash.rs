// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! CLI presentation for explicitly requested native Source hashing.

use crate::{output_cli::ReportFormat, source, utf8_file};
use clap::Args;
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
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

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let result = (|| {
        let path = fs::canonicalize(&options.spec).map_err(|e| e.to_string())?;
        let original = utf8_file::read(&path).map_err(|e| e.to_string())?;
        let result = source::calculate(&path, &original, &[options.source], &options.defines)?;
        source::ensure_unchanged(&path, &original)?;
        Ok::<_, String>((result, path))
    })();
    let valid = result.is_ok();
    let mut stdout = io::stdout().lock();
    if matches!(options.format, ReportFormat::Json) {
        let display_path = options.spec.to_string_lossy();
        let mut report = match result {
            Ok((value, _)) => {
                let mut report = serde_json::to_value(&value.sources[&options.source])
                    .expect("serializable download");
                report["input"] =
                    serde_json::json!({"display_path": display_path, "sha256": value.input_sha256});
                report["native"] =
                    serde_json::to_value(value.native).expect("serializable evidence");
                report
            }
            Err(error) => {
                serde_json::json!({"input": {"display_path": display_path}, "error": error})
            }
        };
        report["format_version"] = 1.into();
        report["valid"] = valid.into();
        report["source"] = options.source.into();
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
