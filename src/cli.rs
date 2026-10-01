// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line syntax and help.

use clap::{Parser, Subcommand};

use crate::{edit, new, source_hash, verify_sources};

#[derive(Parser)]
#[command(version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
#[command(defer = true)]
pub(crate) enum Command {
    /// Initializes workspace configuration, optionally cloning a recipe Git repository.
    Init(crate::workspace::Options),
    /// Builds a prepared SPEC in the embedded openRuyi environment.
    Build(crate::build::Options),
    /// Enters a retained package build inside its Mock chroot.
    Shell(crate::build::shell::Options),
    /// Creates a named package development checkout and authoring scaffold.
    New(new::Options),
    /// Prints editor schemas for manifests or selected editable SPEC fields.
    #[command(subcommand)]
    Schema(crate::schema::Options),
    /// Edits SPEC fields through TOML or command-line assignments.
    Edit(edit::Options),
    /// Calculates or verifies remote Source digests without changing recipes.
    #[command(subcommand)]
    Source(SourceCommand),
    /// Removes retained build results and resources without deleting authoring files.
    Clean(crate::clean::Options),
    /// Checks package metadata and optionally staged Source/Patch files, without downloading.
    Check(crate::check::Options),
    /// Inspects SPEC facts or prints selected editable fields.
    Inspect(crate::inspect::Options),
    /// Completes a bound WORK and optionally publishes its checked SPEC.
    Gen(crate::generate::Options),
}

#[derive(Subcommand)]
pub(crate) enum SourceCommand {
    /// Resolve one Source and calculate its downloaded SHA-256.
    Hash(source_hash::Options),
    /// Compare downloaded Sources with declared digests; never write them back.
    Verify(verify_sources::Options),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn report_format_does_not_mix_with_payload_outputs() {
        let edit_modes: &[&[&str]] = &[&["--stdout"], &["--editor", "vim"]];
        let gen_modes: &[&[&str]] = &[
            &["--stdout"],
            &["--diff"],
            &["--force"],
            &["--skip-existing"],
        ];
        for (command, input, modes) in [("edit", "ed", edit_modes), ("gen", "ed", gen_modes)] {
            let parse = |args: &[&str]| {
                Cli::try_parse_from(
                    ["ruyipack", command, input]
                        .into_iter()
                        .chain(args.iter().copied()),
                )
            };
            for format in ["human", "toml"] {
                assert!(parse(&["--check", "--format", format]).is_ok());
                if command == "gen" {
                    assert_eq!(
                        parse(&["--format", format]).err().map(|error| error.kind()),
                        Some(ErrorKind::MissingRequiredArgument)
                    );
                } else {
                    assert!(parse(&["--diff", "--format", format]).is_ok());
                    for action in [
                        vec!["--prepare", "drafts"],
                        vec!["--set", "package.version=2"],
                        vec!["--apply", "--output", "other.spec", "--force"],
                    ] {
                        let mut args = vec!["--format", format];
                        args.extend(action);
                        assert!(parse(&args).is_ok());
                    }
                }
                for mode in modes {
                    assert!(parse(mode).is_ok(), "{command} {mode:?}");
                    let mut args = vec!["--format", format];
                    args.extend_from_slice(mode);
                    for check in [false, true] {
                        if check {
                            args.push("--check");
                        }
                        assert_eq!(
                            parse(&args).err().map(|error| error.kind()),
                            Some(ErrorKind::ArgumentConflict),
                            "{command} {args:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn generation_output_modes_are_exclusive() {
        for args in [
            ["--force", "--skip-existing"],
            ["--diff", "--force"],
            ["--diff", "--skip-existing"],
            ["--stdout", "--diff"],
            ["--stdout", "--force"],
            ["--stdout", "--skip-existing"],
            ["--check", "--stdout"],
            ["--check", "--diff"],
            ["--check", "--force"],
            ["--check", "--skip-existing"],
            ["--hash", "--offline"],
        ] {
            assert_eq!(
                Cli::try_parse_from(["ruyipack", "gen", "ed"].into_iter().chain(args))
                    .err()
                    .map(|error| error.kind()),
                Some(ErrorKind::ArgumentConflict),
                "{args:?}"
            );
        }
    }
}
