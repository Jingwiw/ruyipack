// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Prepare declared materials without starting a build environment.

use crate::{
    check::materials,
    output_cli::{ReportFormat, report_input},
    workspace::SpecOptions,
};
use clap::Args;
use std::{io, path::PathBuf};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("fetch-input").args(["work", "spec"]).required(true)))]
pub(crate) struct Options {
    #[command(flatten)]
    input: SpecOptions,
    /// Explicit SPEC mode: material directory (defaults to the SPEC's parent).
    #[arg(long, requires = "spec", value_hint = clap::ValueHint::DirPath)]
    source_dir: Option<PathBuf>,
    /// Check available materials without downloading; this does not configure Mock networking.
    #[arg(long)]
    offline: bool,
    /// Define a static macro before resolving material declarations.
    #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
    defines: Vec<String>,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

#[derive(serde::Serialize)]
struct Report<'a> {
    format_version: u32,
    tool: crate::tool::Identity,
    input: crate::report::Input<'a>,
    success: bool,
    materials: Option<&'a materials::Report>,
    error: Option<crate::report::Failure>,
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let Some(input) = report_input(
        options.input.resolve_materials(),
        &options.input.display(),
        options.format,
    )
    .map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };
    let root = options
        .source_dir
        .as_deref()
        .unwrap_or_else(|| input.directory());
    let cache = input.sources().unwrap_or_else(|| root.to_path_buf());
    let parsed = crate::spec::ParsedSpec::parse(input.source.as_str());
    let result = materials::prepare(root, &cache, &parsed, &options.defines, options.offline)
        .and_then(|report| {
            if !input.is_unchanged().map_err(|e| e.to_string())? {
                return Err(
                    "SPEC changed during material preparation; fetched bytes retained, retry"
                        .into(),
                );
            }
            Ok(report)
        });
    if matches!(options.format, ReportFormat::Toml) {
        let hash = crate::utf8_file::sha256(&input.source);
        crate::report::write(
            &mut io::stdout().lock(),
            &Report {
                format_version: 1,
                tool: crate::tool::identity(),
                input: crate::report::Input {
                    display_path: input.path.to_string_lossy(),
                    sha256: Some(&hash),
                    revision: input.revision.as_deref(),
                },
                success: result.is_ok(),
                materials: result.as_ref().ok(),
                error: result
                    .as_ref()
                    .err()
                    .map(|e| crate::report::failure("material-preparation", e)),
            },
        )
        .map_err(|e| e.to_string())?;
        return Ok(result.is_ok());
    }
    result?
        .write_human(&mut io::stdout().lock())
        .map_err(|e| e.to_string())?;
    Ok(true)
}
