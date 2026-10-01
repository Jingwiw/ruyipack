// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only SPEC facts and source-mapped editable projections.

use clap::Args;
use std::io::{self, Write};

use crate::{
    output_cli::{ReportError, ReportFormat, report_input},
    spec::{ParsedSpec, document::Snapshot, inspection::Inspection},
    workspace::SpecOptions,
};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("inspect-input").required(true).args(["work", "spec"])))]
#[command(group(clap::ArgGroup::new("projection-scope").args(["fields", "all"])))]
pub(crate) struct Options {
    #[command(flatten)]
    pub(crate) input: SpecOptions,
    /// Show parser facts as human-readable text or a machine report.
    #[arg(long, value_enum, default_value = "human")]
    pub(crate) format: ReportFormat,
    /// Print the editable TOML projection without editing or validating package policy.
    #[arg(long, requires = "projection-scope", conflicts_with = "format")]
    pub(crate) editable: bool,
    /// Select an editable field or group; repeat to add fields.
    #[arg(
        long = "field",
        value_name = "FIELD",
        requires = "editable",
        conflicts_with = "all"
    )]
    pub(crate) fields: Vec<String>,
    /// Include all supported editable fields; unsupported constructs are rejected.
    #[arg(long, requires = "editable")]
    pub(crate) all: bool,
}

/// Read-only projection rejects ambiguous mappings; ordinary inspection retains diagnostics.
pub(crate) fn run(options: &Options) -> Result<bool, ReportError> {
    let Some(input) = report_input(
        options.input.resolve(),
        &options.input.display(),
        options.format,
    )?
    else {
        return Ok(false);
    };
    let parsed = ParsedSpec::parse(&input.source);
    let mut output = io::stdout().lock();
    if options.editable {
        let snapshot = Snapshot::capture_selected(&parsed, &options.fields).map_err(|error| {
            ReportError::Projection(format!("{}: {error}", input.path.display()))
        })?;
        let document = toml::to_string_pretty(snapshot.document())
            .map_err(|error| ReportError::Projection(error.to_string()))?;
        output
            .write_all(document.as_bytes())
            .map_err(ReportError::Stdout)?;
    } else {
        let view = Inspection::new(parsed);
        match options.format {
            ReportFormat::Human => {
                view.write_diagnostics(&input.path, &mut crate::output_cli::stderr())
                    .map_err(ReportError::Stderr)?;
                view.write_human(&mut output)
            }
            ReportFormat::Toml => {
                view.write_toml(&input.path, input.revision.as_deref(), &mut output)
            }
        }
        .map_err(ReportError::Stdout)?;
    }
    Ok(true)
}
