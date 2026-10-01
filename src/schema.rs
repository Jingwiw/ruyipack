// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Editor schemas for authoring manifests and a SPEC's fixed editable projection.

use clap::{Args, Subcommand};
use std::io::{self, Write};

use crate::{
    output_cli::ReportError,
    spec::{
        ParsedSpec,
        document::{Snapshot, schema},
    },
    workspace::SpecOptions,
};

#[derive(Subcommand)]
pub(crate) enum Options {
    /// Print the authoring manifest schema.
    Manifest,
    /// Print the schema of selected fields from one SPEC.
    Edit(EditOptions),
}

#[derive(Args)]
#[command(group(clap::ArgGroup::new("schema-input").required(true).args(["work", "spec"])))]
#[command(group(clap::ArgGroup::new("schema-scope").required(true).args(["fields", "all"])))]
pub(crate) struct EditOptions {
    #[command(flatten)]
    pub(crate) input: SpecOptions,
    /// Select an editable field or group; repeat to add fields.
    #[arg(long = "field", value_name = "FIELD", conflicts_with = "all")]
    pub(crate) fields: Vec<String>,
    /// Include all supported editable fields; unsupported constructs are rejected.
    #[arg(long)]
    pub(crate) all: bool,
}

pub(crate) fn run(options: &Options) -> Result<bool, ReportError> {
    let schema = match options {
        Options::Manifest => crate::render::manifest::schema::generate().to_value(),
        Options::Edit(options) => {
            let input = options.input.resolve()?;
            let parsed = ParsedSpec::parse(&input.source);
            let selection = if options.all {
                &[][..]
            } else {
                &options.fields
            };
            let snapshot = Snapshot::capture_selected(&parsed, selection).map_err(|error| {
                ReportError::Projection(format!("{}: {error}", input.path.display()))
            })?;
            schema::generate(snapshot.document())
        }
    };
    writeln!(io::stdout().lock(), "{schema:#}").map_err(ReportError::Stdout)?;
    Ok(true)
}
