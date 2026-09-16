// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Read-only previews of checked, source-preserving SPEC edits.

mod document;
mod fields;

use std::{
    io::{self, Write},
    path::PathBuf,
};

use clap::{ArgGroup, Args};
use rpm_spec::parser::parse_str_with_spans;

use crate::{check, file_output, utf8_file};
use document::Snapshot;

#[derive(Args)]
#[command(
    group(ArgGroup::new("action").required(true).args(["view", "set"])),
    after_help = "--view prints editable TOML. --set prints a checked diff by default.\nThese previews never write SPEC files."
)]
pub(crate) struct Options {
    /// SPEC file to preview.
    #[arg(value_name = "SPEC")]
    spec: PathBuf,
    /// Prints the editable TOML without changing the SPEC.
    #[arg(long, conflicts_with = "diff")]
    view: bool,
    /// Sets one existing string field; repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE", num_args = 1, value_parser = assignment)]
    set: Vec<(String, String)>,
    /// Explicitly selects the default read-only diff for --set.
    #[arg(long, requires = "set")]
    diff: bool,
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let source = utf8_file::read(&options.spec).map_err(|error| error.to_string())?;
    let parsed = parse_str_with_spans(&source);
    let snapshot = Snapshot::capture(&source, &parsed)?;
    if options.view {
        let text =
            toml::to_string_pretty(snapshot.document()).map_err(|error| error.to_string())?;
        io::stdout()
            .lock()
            .write_all(text.as_bytes())
            .map_err(|error| format!("failed to write output to stdout: {error}"))?;
        return Ok(true);
    }
    if options.diff || !options.set.is_empty() {
        let edited = fields::assign(snapshot.document(), &options.set)?;
        let candidate = snapshot.render(&edited)?;
        let parsed = parse_str_with_spans(&candidate);
        let recovered = Snapshot::capture(&candidate, &parsed)?;
        if recovered.document() != &edited {
            return Err("edited fields did not survive SPEC parsing".into());
        }
        let report = check::analyze(&candidate, parsed);
        report
            .write_human(&options.spec, &mut io::stderr().lock())
            .map_err(|error| format!("failed to write diagnostics to stderr: {error}"))?;
        if !report.is_success() {
            return Err("candidate failed static checks".into());
        }
        file_output::show_edit_diff(&options.spec, &source, &candidate)
            .map_err(|error| error.to_string())?;
        return Ok(true);
    }
    Err("choose --view or --set FIELD=VALUE".into())
}

fn assignment(text: &str) -> Result<(String, String), String> {
    let (field, value) = text.split_once('=').ok_or("expected FIELD=VALUE")?;
    if field.is_empty() {
        return Err("FIELD cannot be empty".into());
    }
    Ok((field.to_owned(), value.to_owned()))
}
