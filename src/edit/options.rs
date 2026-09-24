// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Editing arguments and output selection.

use clap::{ArgGroup, Args, ValueEnum};
use std::path::PathBuf;

#[derive(Args)]
#[command(
    group(ArgGroup::new("edit_action").args([
        "prepare", "view", "schema", "check", "diff", "stdout", "output"
    ])),
    after_help = "Opens selected SPEC fields as TOML in $VISUAL, $EDITOR, or vim.\nUse --field to edit part of a SPEC; a full view requires all constructs to be supported.\nUse --set FIELD=VALUE repeatedly for string fields.\nUse --prepare DIR for persistent drafts.\nAfter the editor exits, checked edits are written to the source SPEC files.\nUse --diff to preview without writing; --from DIR applies saved drafts.\nChanging Version or Source does not refresh recorded digests or verify patches.\nReview them before building or submitting the package.\nExamples:\n  ruyipack edit ed.spec --set package.version=1.22 --diff\n  ruyipack edit ed.spec --field package.version --editor 'code --wait'\n  ruyipack edit ed.spec make.spec --field package.version --prepare drafts\n  ruyipack edit --from drafts --check\n  ruyipack edit --from drafts --diff\n  ruyipack edit --from drafts"
)]
pub(crate) struct Options {
    /// SPEC files to edit.
    #[arg(
        value_name = "SPEC",
        required_unless_present = "from",
        conflicts_with = "from"
    )]
    pub specs: Vec<PathBuf>,
    /// Applies saved drafts without opening an editor; --editor reopens them.
    #[arg(long, value_name = "DIR", conflicts_with_all = ["prepare", "set", "field", "view", "schema"])]
    pub from: Option<PathBuf>,
    /// Writes editable TOML files and their source bindings to a directory.
    #[arg(long, value_name = "DIR", conflicts_with_all = ["set", "editor"])]
    pub prepare: Option<PathBuf>,
    /// Sets one existing string field; repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE", value_parser = assignment, conflicts_with_all = ["field", "view", "schema", "editor"])]
    pub set: Vec<(String, String)>,
    /// Selects a field or table for viewing or editing; repeat to add fields.
    #[arg(long, value_name = "FIELD")]
    pub field: Vec<String>,
    /// Prints the editable TOML for one SPEC without opening an editor.
    #[arg(long, conflicts_with = "editor")]
    pub view: bool,
    /// Prints a JSON Schema for the displayed fields of one SPEC.
    #[arg(long, conflicts_with = "editor")]
    pub schema: bool,
    /// Checks all drafts without writing SPEC files or opening an editor.
    #[arg(long, conflicts_with = "editor")]
    pub check: bool,
    /// Selects the check report format.
    // Conflicts can waive `requires`, so reject non-check modes on this option too.
    #[arg(long, value_enum, requires = "check", conflicts_with_all = ["prepare", "view", "schema", "diff", "stdout", "output", "editor", "force"])]
    pub format: Option<CheckFormat>,
    /// Prints source-to-candidate diffs without writing SPEC files.
    #[arg(long)]
    pub diff: bool,
    /// Prints one checked SPEC without writing a file.
    #[arg(long)]
    pub stdout: bool,
    /// Replaces an existing --output file without prompting.
    // Clap waives required arguments when a conflicting group member is present.
    #[arg(long, requires = "output", conflicts_with_all = ["prepare", "view", "schema", "check", "diff", "stdout"])]
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

#[cfg(test)]
mod tests {
    use clap::{Parser, error::ErrorKind};

    use crate::Cli;

    #[test]
    fn edit_actions_are_optional_exclusive_and_force_requires_output() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(["ruyipack", "edit"].into_iter().chain(args.iter().copied()))
        };
        let actions: [&[&str]; 7] = [
            &["--prepare", "drafts"],
            &["--view"],
            &["--schema"],
            &["--check"],
            &["--diff"],
            &["--stdout"],
            &["--output", "other.spec"],
        ];
        assert!(parse(&["ed.spec"]).is_ok());
        assert!(parse(&["ed.spec", "--force"]).is_err());
        for (index, action) in actions.iter().enumerate() {
            let mut args = vec!["ed.spec"];
            args.extend_from_slice(action);
            assert!(parse(&args).is_ok(), "{args:?}");
            args.push("--force");
            assert_eq!(parse(&args).is_ok(), action[0] == "--output", "{args:?}");
            args.pop();
            for other in &actions[index + 1..] {
                let mut pair = args.clone();
                pair.extend_from_slice(other);
                assert_eq!(
                    parse(&pair).err().map(|error| error.kind()),
                    Some(ErrorKind::ArgumentConflict),
                    "{pair:?}"
                );
            }
        }
    }
}
