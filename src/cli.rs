// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line syntax and help.

use clap::{CommandFactory, Parser, Subcommand};

use crate::{edit, new, source_fetch, source_hash, verify_sources};

#[derive(Parser)]
#[command(
    version,
    about,
    after_help = "Use the same WORK name with open, edit, check, build and shell.",
    after_long_help = "Existing SPEC: open/edit WORK -> check WORK -> build WORK -> shell WORK.\nTOML recipe: new WORK -> open WORK --authoring -> gen WORK --apply -> build WORK.\nclean WORK keeps your package changes; delete WORK removes the development area.\nEditor tools: schema manifest; schema edit WORK --field FIELD.\nShell setup: completions zsh."
)]
pub(crate) struct Cli {
    /// Show internal diagnostics.
    #[arg(long, global = true)]
    pub(crate) debug: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
#[command(defer = true)]
pub(crate) enum Command {
    /// Initialize a packaging workspace.
    Init(crate::workspace::Options),
    /// Start working on a package.
    New(new::Options),
    /// Open package files in your editor.
    Open(crate::open::Options),
    /// Change package metadata and source checksums.
    Edit(edit::Options),
    /// Generate a SPEC from TOML.
    Gen(crate::generate::Options),
    /// Check a package; optionally apply supported repairs.
    Check(crate::check::Options),
    /// Build the recipe SPEC and retain the results.
    Build(crate::build::Options),
    /// Submit local package materials to an OBS home project.
    RemoteBuild(crate::remote_build::Options),
    /// Enter the retained build environment.
    Shell(crate::build::shell::Options),
    /// Download sources or verify their checksums.
    #[command(subcommand)]
    Source(SourceCommand),
    /// Show package information.
    Inspect(crate::inspect::Options),
    /// Commit package changes to the configured Git repository.
    Commit(crate::workspace::commit::Options),
    /// Preview or publish a PR for committed packages in a plan.
    Pr(crate::workspace::pr::Options),
    /// Remove build results; keep package changes.
    Clean(crate::clean::Options),
    /// Remove a development area.
    Delete(crate::workspace::delete::Options),
    /// Print editor schemas for authoring TOML or selected SPEC fields.
    #[command(subcommand, hide = true)]
    Schema(crate::schema::Options),
    /// Print a shell completion script.
    #[command(hide = true)]
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

/// Generate from the same command tree used to parse arguments.
pub(crate) fn completions(
    shell: clap_complete::Shell,
) -> Result<bool, crate::output_cli::ReportError> {
    use clap_complete::Generator;

    let mut command = Cli::command().bin_name("ruyipack");
    command.build();
    shell
        .try_generate(&command, &mut std::io::stdout().lock())
        .map_err(crate::output_cli::ReportError::Stdout)?;
    Ok(true)
}

#[derive(Subcommand)]
pub(crate) enum SourceCommand {
    /// Fetch missing Source/Patch materials and verify declared checksums, without building.
    Fetch(source_fetch::Options),
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
        let gen_modes: &[&[&str]] = &[&["--stdout"]];
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
                assert!(parse(&["--format", format]).is_ok());
                assert!(parse(&["--diff", "--format", format]).is_ok());
                assert!(parse(&["--apply", "--format", format]).is_ok());
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
            ["--apply", "--output=other.spec"],
            ["--apply", "--check"],
            ["--apply", "--stdout"],
            ["--force", "--skip-existing"],
            ["--stdout", "--diff"],
            ["--stdout", "--force"],
            ["--stdout", "--skip-existing"],
            ["--check", "--stdout"],
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
