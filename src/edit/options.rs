// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Editing arguments and output selection.

use crate::output_cli::ReportFormat;
use clap::Args;
use std::path::PathBuf;

#[derive(Args, Default)]
#[command(
    after_help = "Edit selected SPEC fields in a TOML draft by default. The recipe stays unchanged until --apply.
--set FIELD=VALUE replaces values; --add FIELD=VALUE appends a new line. --field and --menu edit values in the terminal.
Lists are edited one item at a time. Unchanged terminal values create no draft.
--diff and --apply use saved edits when no new values are given; no editor opens.
--diff shows changes. --check checks the candidate. --apply checks and writes the SPEC.
--hash refreshes remote Source digests using the candidate version and URLs.
--menu offers conf, then -p (prepend), -a (append), or replace.
Use --set build.stages.conf.prepend='SCRIPT' for the same noninteractive edit.
Unchanged issues can remain. New errors and incomplete checks block publication.
Example: ruyipack edit ed --set package.version=1.22.6 --hash --diff --apply"
)]
pub(crate) struct Options {
    /// Development areas to edit; repeat for a batch.
    #[arg(value_name = "WORK", required_unless_present_any = ["specs", "from"], conflicts_with_all = ["specs", "from"])]
    pub works: Vec<String>,
    /// Internal check repair: preserve declared digests and refuse pending edits.
    #[arg(skip)]
    pub repair_missing: bool,
    #[arg(skip)]
    pub upgrade: Option<Box<crate::check::upgrade::Report>>,
    /// Edit explicit SPEC files instead of development areas.
    #[arg(long = "spec", value_name = "PATH", conflicts_with_all = ["works", "from", "pkgname"])]
    pub specs: Vec<PathBuf>,
    /// Package binding for a new development area.
    #[arg(long, value_name = "PKG", requires = "works")]
    pub pkgname: Option<String>,
    /// Resume saved TOML draft values; --editor reopens it.
    #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, conflicts_with_all = ["prepare", "set", "add", "fields", "menu"], help_heading = "Advanced options", hide_short_help = true)]
    pub from: Option<PathBuf>,
    /// Prepare a persistent draft in this directory without opening the editor.
    #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, conflicts_with_all = ["editor", "menu"], help_heading = "Advanced options", hide_short_help = true)]
    pub prepare: Option<PathBuf>,
    /// Choose fields from a menu and edit their existing values inline.
    #[arg(long, conflicts_with_all = ["set", "add", "editor", "all"])]
    pub menu: bool,
    /// Set an existing string or TOML string-array field; repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE", value_parser = assignment, conflicts_with_all = ["fields", "editor"])]
    pub set: Vec<(String, String)>,
    /// Append text on a new line to a string field; repeat to append more text.
    #[arg(long, value_name = "FIELD=VALUE", value_parser = assignment, conflicts_with_all = ["fields", "editor"])]
    pub add: Vec<(String, String)>,
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
    #[arg(long, value_name = "HASH", value_parser = parse_expected_sha256, help_heading = "Advanced options", hide_short_help = true)]
    pub expect_sha256: Option<String>,
    /// Edit a field or table inline; --editor or --prepare instead uses TOML.
    #[arg(long = "field", value_name = "FIELD")]
    pub fields: Vec<String>,
    /// Require a complete mapping of every construct instead of a safe projection.
    #[arg(long, conflicts_with_all = ["fields", "set", "add", "from"], help_heading = "Advanced options", hide_short_help = true)]
    pub all: bool,
    /// Check local-edit admission after editing; does not publish by itself.
    #[arg(long)]
    pub check: bool,
    /// Explicitly publish the candidate after local-edit admission succeeds.
    #[arg(long)]
    pub apply: bool,
    /// Select the draft, check, or publication report format.
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
    /// Override the configured editor for TOML; GUI editors must wait.
    #[arg(long, value_name = "COMMAND")]
    pub editor: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Interaction {
    Editor,
    Inline,
    None,
}

impl Options {
    pub(super) fn interaction(&self) -> Interaction {
        if self.editor.is_some() {
            Interaction::Editor
        } else if self.prepare.is_some()
            || self.from.is_some()
            || !self.set.is_empty()
            || !self.add.is_empty()
        {
            Interaction::None
        } else if self.menu || !self.fields.is_empty() {
            Interaction::Inline
        } else if self.generates_candidate() {
            Interaction::None
        } else {
            Interaction::Editor
        }
    }

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
            && self.add.is_empty()
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
