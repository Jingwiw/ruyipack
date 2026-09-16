// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Editing arguments and output selection.

use clap::{Args, ValueEnum};
use std::path::PathBuf;

#[derive(Args)]
#[command(
    after_help = "With no editing option, opens TOML in $VISUAL, $EDITOR, or vim.\nUse --set FIELD=VALUE repeatedly for string fields.\nUse --prepare DIR for persistent drafts; --from DIR reads them back.\nSaving TOML does not overwrite SPEC files. --force applies checked edits.\nExamples:\n  ruyipack edit ed.spec --set package.version=1.22 --diff\n  ruyipack edit ed.spec --editor 'code --wait'\n  ruyipack edit ed.spec make.spec --prepare drafts\n  ruyipack edit --from drafts --check\n  ruyipack edit --from drafts --diff\n  ruyipack edit --from drafts --force"
)]
pub(crate) struct Options {
    /// SPEC files to edit.
    #[arg(
        value_name = "SPEC",
        required_unless_present = "from",
        conflicts_with = "from"
    )]
    pub specs: Vec<PathBuf>,
    /// Reads a directory previously created by --prepare.
    #[arg(long, value_name = "DIR", conflicts_with_all = ["prepare", "set", "field", "view", "schema"])]
    pub from: Option<PathBuf>,
    /// Writes editable TOML files and their source bindings to a directory.
    #[arg(long, value_name = "DIR", conflicts_with_all = ["set", "view", "schema", "check", "diff", "stdout", "force", "output", "editor"])]
    pub prepare: Option<PathBuf>,
    /// Sets one existing string field; repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE", value_parser = assignment, conflicts_with_all = ["field", "view", "schema", "editor"])]
    pub set: Vec<(String, String)>,
    /// Selects a field or table for viewing or editing; repeat to add fields.
    #[arg(long, value_name = "FIELD")]
    pub field: Vec<String>,
    /// Prints the editable TOML for one SPEC without opening an editor.
    #[arg(long, conflicts_with_all = ["schema", "check", "diff", "stdout", "force", "output", "editor"])]
    pub view: bool,
    /// Prints a JSON Schema for the displayed fields of one SPEC.
    #[arg(long, conflicts_with_all = ["check", "diff", "stdout", "force", "output", "editor"])]
    pub schema: bool,
    /// Checks all drafts without writing SPEC files or opening an editor.
    #[arg(long, conflicts_with_all = ["diff", "stdout", "force", "output", "editor"])]
    pub check: bool,
    /// Selects the check report format.
    #[arg(long, value_enum, requires = "check")]
    pub format: Option<CheckFormat>,
    /// Prints source-to-candidate diffs without writing SPEC files.
    #[arg(long, conflicts_with_all = ["stdout", "force", "output"])]
    pub diff: bool,
    /// Prints one checked SPEC without writing a file.
    #[arg(long, conflicts_with_all = ["force", "output"])]
    pub stdout: bool,
    /// Applies checked edits without a confirmation menu.
    #[arg(long)]
    pub force: bool,
    /// Writes one edited SPEC to this path instead of replacing its source.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Overrides the editor command; GUI editors must wait until files close.
    #[arg(long, value_name = "COMMAND")]
    pub editor: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum CheckFormat {
    Human,
    Json,
}

fn assignment(text: &str) -> Result<(String, String), String> {
    let (field, value) = text.split_once('=').ok_or("expected FIELD=VALUE")?;
    if field.is_empty() {
        return Err("FIELD cannot be empty".into());
    }
    Ok((field.to_owned(), value.to_owned()))
}
