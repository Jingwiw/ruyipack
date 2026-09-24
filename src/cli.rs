// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line syntax and help.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{check_command, edit, init, inspect, output_cli};

#[derive(Parser)]
#[command(version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
#[command(defer = true)]
pub(crate) enum Command {
    /// Creates a package manifest template in an existing directory.
    Init(init::Options),
    /// Edits SPEC fields through TOML or command-line assignments.
    Edit(edit::Options),
    /// Checks selected static package metadata and build requirements.
    Check {
        /// RPM SPEC file to check.
        #[arg(value_name = "SPEC")]
        spec: PathBuf,
        /// Selects human or JSON output.
        #[arg(long, value_enum, default_value_t = check_command::CheckFormat::Human)]
        format: check_command::CheckFormat,
    },
    /// Prints the normalized main-package tags from an RPM SPEC file.
    Inspect {
        /// RPM SPEC file to inspect.
        #[arg(value_name = "SPEC")]
        spec: PathBuf,
        /// Selects human or JSON output.
        #[arg(long, value_enum, default_value_t = inspect::InspectFormat::Human)]
        format: inspect::InspectFormat,
    },
    /// Generates an openRuyi SPEC from a package manifest.
    #[command(
        after_help = "The default output is NAME.spec beside the manifest. Its parent directory must exist.\n\
For different existing content, select an output option or use the terminal menu.\n\
Without a usable terminal or an explicit action, conflicting output is an error."
    )]
    Gen {
        /// Package to generate.
        #[arg(value_name = "NAME")]
        name: String,
        /// Manifest to read; defaults to NAME.toml in the current directory.
        #[arg(long, value_name = "PATH")]
        manifest: Option<PathBuf>,
        /// Checks generation without writing SPEC files or consulting output conflicts.
        #[arg(long, conflicts_with_all = ["path", "stdout", "diff", "force", "skip_existing"])]
        check: bool,
        /// Selects the generation check report format.
        // Conflicts can waive `requires`, so reject non-check modes on this option too.
        #[arg(long, value_enum, requires = "check", conflicts_with_all = ["path", "stdout", "diff", "force", "skip_existing"])]
        format: Option<check_command::CheckFormat>,
        #[command(flatten, next_help_heading = "Output options")]
        output: output_cli::OutputOptions,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn report_format_requires_an_actual_check() {
        let edit_modes: &[&[&str]] = &[
            &["--prepare", "drafts"],
            &["--view"],
            &["--schema"],
            &["--diff"],
            &["--stdout"],
            &["--output", "other.spec"],
            &["--editor", "vim"],
            &["--output", "other.spec", "--force"],
        ];
        let gen_modes: &[&[&str]] = &[
            &["--output", "other.spec"],
            &["--stdout"],
            &["--diff"],
            &["--force"],
            &["--skip-existing"],
        ];
        for (command, input, modes) in [("edit", "ed.spec", edit_modes), ("gen", "ed", gen_modes)] {
            let parse = |args: &[&str]| {
                Cli::try_parse_from(
                    ["ruyipack", command, input]
                        .into_iter()
                        .chain(args.iter().copied()),
                )
            };
            for format in ["human", "json"] {
                assert!(parse(&["--check", "--format", format]).is_ok());
                assert_eq!(
                    parse(&["--format", format]).err().map(|error| error.kind()),
                    Some(ErrorKind::MissingRequiredArgument)
                );
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
}
