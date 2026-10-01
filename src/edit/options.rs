// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Editing arguments and output selection.

use crate::output_cli::ReportFormat;
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
#[command(
    after_help = "Default: open a persistent TOML stage in $VISUAL, $EDITOR, or vim.
--menu selects fields, then edits their values inline; --field edits known fields inline.
--set FIELD=VALUE is non-interactive. Use --field FIELD --editor COMMAND for selected TOML editing.
Neither mode writes SPEC files by default.
--check checks the edit; --diff caches a candidate SPEC and saves/displays its diff.
--apply checks local-edit admission and explicitly publishes to the development checkout.
--check, --diff and --apply may be combined. Unchanged confirmed legacy issues may remain.
--hash refreshes every remote Source (including signatures, excluding local files).
--hash-source N refreshes only selected Sources. Hashes use the edited candidate and return to its stage.
Use --from DIR to resume a stage; --editor reopens its TOML.
Examples:
  ruyipack edit ed
  ruyipack edit ed --menu --check
  ruyipack edit ed --field package.version --diff
  ruyipack edit ed --set package.version=1.22 --diff
  ruyipack edit ed --set package.version=1.22 --hash --check --diff --apply
  ruyipack edit --from work/ed/stage --apply"
)]
pub(crate) struct Options {
    /// Development areas to edit; repeat for a batch.
    #[arg(value_name = "WORK", required_unless_present_any = ["specs", "from"], conflicts_with_all = ["specs", "from"])]
    pub works: Vec<String>,
    /// Edit explicit SPEC files instead of development areas.
    #[arg(long = "spec", value_name = "PATH", conflicts_with_all = ["works", "from", "pkgname"])]
    pub specs: Vec<PathBuf>,
    /// Package binding for a new development area.
    #[arg(long, value_name = "PKG", requires = "works")]
    pub pkgname: Option<String>,
    /// Resume saved TOML stage values; --editor reopens it.
    #[arg(long, value_name = "DIR", conflicts_with_all = ["prepare", "set", "fields", "menu"])]
    pub from: Option<PathBuf>,
    /// Prepare a persistent stage in this directory without opening the editor.
    #[arg(long, value_name = "DIR", conflicts_with_all = ["editor", "menu"])]
    pub prepare: Option<PathBuf>,
    /// Choose fields from a menu and edit their existing values inline.
    #[arg(long, conflicts_with_all = ["set", "editor", "all"])]
    pub menu: bool,
    /// Set an existing string or TOML string-array field; repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE", value_parser = assignment, conflicts_with_all = ["fields", "editor"])]
    pub set: Vec<(String, String)>,
    /// Refresh SHA-256 for all remote Sources, including signature Sources.
    #[arg(long)]
    pub hash: bool,
    /// Refresh the SHA-256 of one Source from the edited candidate.
    #[arg(long = "hash-source", value_name = "N")]
    pub hash_sources: Vec<u32>,
    /// Define a static macro before resolving the edited candidate.
    #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
    pub defines: Vec<String>,
    /// Refuse an operation unless its original SHA-256 matches.
    #[arg(long, value_name = "HASH", value_parser = parse_expected_sha256)]
    pub expect_sha256: Option<String>,
    /// Edit a field or table inline; --editor or --prepare instead uses TOML.
    #[arg(long = "field", value_name = "FIELD")]
    pub fields: Vec<String>,
    /// Require a complete mapping of every construct instead of a safe projection.
    #[arg(long, conflicts_with_all = ["fields", "set", "from"])]
    pub all: bool,
    /// Check local-edit admission after editing; does not publish by itself.
    #[arg(long)]
    pub check: bool,
    /// Explicitly publish the candidate after local-edit admission succeeds.
    #[arg(long)]
    pub apply: bool,
    /// Select the stage, check, or publication report format.
    #[arg(long, value_enum, conflicts_with_all = ["stdout", "editor", "menu"])]
    pub format: Option<ReportFormat>,
    /// Cache the candidate SPEC and save/display its unified diff; no implicit check.
    #[arg(long)]
    pub diff: bool,
    /// Print one constructed candidate SPEC without publishing it.
    #[arg(long, conflicts_with = "diff")]
    pub stdout: bool,
    /// Replace an existing --output destination without prompting.
    #[arg(long, requires = "output")]
    pub force: bool,
    /// With --apply, publish to this file instead of replacing the source.
    #[arg(
        short,
        long,
        value_name = "FILE",
        requires = "apply",
        conflicts_with = "stdout"
    )]
    pub output: Option<PathBuf>,
    /// Override the TOML editor command; GUI editors must wait.
    #[arg(long, value_name = "COMMAND")]
    pub editor: Option<String>,
}

impl Options {
    pub(super) fn generates_candidate(&self) -> bool {
        self.check
            || self.apply
            || self.diff
            || self.stdout
            || self.hash
            || !self.hash_sources.is_empty()
    }

    pub(super) fn checks_only(&self) -> bool {
        self.check
            && !self.apply
            && !self.diff
            && self.prepare.is_none()
            && self.from.is_none()
            && self.set.is_empty()
            && self.fields.is_empty()
            && !self.menu
            && !self.hash
            && self.hash_sources.is_empty()
            && self.editor.is_none()
    }
}

fn assignment(text: &str) -> Result<(String, String), String> {
    let (field, value) = text.split_once('=').ok_or("expected FIELD=VALUE")?;
    if field.is_empty() {
        return Err("FIELD cannot be empty".into());
    }
    Ok((field.to_owned(), value.to_owned()))
}

fn parse_expected_sha256(value: &str) -> Result<String, &'static str> {
    crate::source::validate_sha256(value)?;
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use crate::Cli;
    use clap::Parser;

    #[test]
    fn edit_operations_are_orthogonal_and_apply_authorizes_output() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(["ruyipack", "edit"].into_iter().chain(args.iter().copied()))
        };
        assert!(parse(&["ed"]).is_ok());
        assert!(parse(&["ed", "--check", "--diff", "--apply", "--hash"]).is_ok());
        assert!(parse(&["ed", "--set", "package.version=2", "--check", "--diff"]).is_ok());
        assert!(parse(&["ed", "--menu", "--check", "--apply"]).is_ok());
        assert!(parse(&["ed", "--output", "copy.spec"]).is_err());
        assert!(parse(&["ed", "--apply", "--output", "copy.spec", "--force"]).is_ok());
        assert!(parse(&["ed", "--force"]).is_err());
    }
}
