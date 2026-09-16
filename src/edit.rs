// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Checked, source-preserving SPEC edits with explicit publication choices.

mod document;
mod fields;

use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};

use clap::{ArgGroup, Args};
use rpm_spec::parser::parse_str_with_spans;

use crate::{check, file_output, utf8_file};
use document::Snapshot;

#[derive(Args)]
#[command(
    group(ArgGroup::new("action").required(true).args(["view", "schema", "set"])),
    group(ArgGroup::new("view_action").args(["view", "schema"])),
    after_help = "--view prints editable TOML; --schema describes its fields.\nUse --field with either view to select fields or groups. --set prepares a checked edit.\nChoose --diff or --stdout for a read-only preview, --force to overwrite,\nor -o FILE for another destination. Conflicts otherwise use a confirmation menu."
)]
pub(crate) struct Options {
    /// SPEC file to edit.
    #[arg(value_name = "SPEC")]
    spec: PathBuf,
    /// Prints the editable TOML without changing the SPEC.
    #[arg(long, conflicts_with = "publication")]
    view: bool,
    /// Prints a JSON Schema for the displayed editable fields.
    #[arg(long, conflicts_with = "publication")]
    schema: bool,
    /// Selects a field or table for --view or --schema; repeat to add fields.
    #[arg(
        long,
        value_name = "FIELD",
        requires = "view_action",
        conflicts_with = "set"
    )]
    field: Vec<String>,
    /// Sets one existing string field; repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE", num_args = 1, value_parser = assignment)]
    set: Vec<(String, String)>,
    #[command(flatten)]
    output: PublicationOptions,
}

#[derive(Args)]
#[command(group(ArgGroup::new("publication").multiple(true).args(["diff", "stdout", "force", "output"])))]
struct PublicationOptions {
    /// Prints a source-to-candidate diff without writing files.
    #[arg(long, requires = "set", conflicts_with_all = ["stdout", "force", "output"])]
    diff: bool,
    /// Prints the checked SPEC without writing files.
    #[arg(long, requires = "set", conflicts_with_all = ["force", "output"])]
    stdout: bool,
    /// Applies the checked edit without a confirmation menu.
    #[arg(long, requires = "set")]
    force: bool,
    /// Writes to another destination instead of replacing the source.
    #[arg(short = 'o', long = "output", value_name = "FILE", requires = "set")]
    output: Option<PathBuf>,
}

pub(crate) fn run(options: &Options) -> Result<bool, String> {
    let source_path = fs::canonicalize(&options.spec)
        .map_err(|error| format!("{}: {error}", options.spec.display()))?;
    let source = utf8_file::read(&source_path).map_err(|error| error.to_string())?;
    let parsed = parse_str_with_spans(&source);
    let snapshot = Snapshot::capture(&source, &parsed)?;
    if options.view || options.schema {
        let selected = fields::select(snapshot.document(), &options.field)?;
        let text = if options.schema {
            serde_json::to_string_pretty(&fields::schema(&selected))
                .map_err(|error| error.to_string())?
                + "\n"
        } else {
            toml::to_string_pretty(&selected).map_err(|error| error.to_string())?
        };
        io::stdout()
            .lock()
            .write_all(text.as_bytes())
            .map_err(|error| format!("failed to write output to stdout: {error}"))?;
        return Ok(true);
    }
    if !options.set.is_empty() {
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
        let mode = if options.output.diff {
            file_output::EditMode::Diff
        } else if options.output.stdout {
            file_output::EditMode::Stdout
        } else if options.output.force {
            file_output::EditMode::Overwrite
        } else {
            file_output::EditMode::Prompt
        };
        file_output::run_edits(
            &[file_output::EditFile {
                source_path: &source_path,
                original: &source,
                target_path: options.output.output.as_deref().unwrap_or(&source_path),
                contents: &candidate,
            }],
            mode,
        )
        .map_err(|error| error.to_string())?;
        return Ok(true);
    }
    Err("choose --view, --schema or --set FIELD=VALUE".into())
}

fn assignment(text: &str) -> Result<(String, String), String> {
    let (field, value) = text.split_once('=').ok_or("expected FIELD=VALUE")?;
    if field.is_empty() {
        return Err("FIELD cannot be empty".into());
    }
    Ok((field.to_owned(), value.to_owned()))
}
