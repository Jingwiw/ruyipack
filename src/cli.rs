// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line syntax and help.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{
    edit, init,
    output_cli::{self, ReportFormat},
    source_hash, verify_sources,
};

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
    /// Prints the authoring manifest JSON Schema for offline editor completion and structural checks.
    Schema,
    /// Edits SPEC fields through TOML or command-line assignments.
    Edit(edit::Options),
    /// Resolves a Source statically and reports the SHA-256 of its downloaded bytes.
    SourceHash(source_hash::Options),
    /// Downloads remote Sources and compares declared digests without changing input files.
    VerifySources(verify_sources::Options),
    /// Checks selected static package metadata and build requirements.
    Check {
        /// Static admission policy; neither policy verifies source bytes or native builds.
        #[arg(long, value_enum, default_value_t = crate::check::Policy::Authoring)]
        policy: crate::check::Policy,
        /// Define a static Source macro; other rules still check unevaluated syntax.
        #[arg(short = 'D', long = "define", value_name = "MACRO EXPR")]
        defines: Vec<String>,
        /// RPM SPEC file to check.
        #[arg(value_name = "SPEC")]
        spec: PathBuf,
        /// Selects human or JSON output.
        #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
        format: ReportFormat,
    },
    /// Prints the normalized main-package tags from an RPM SPEC file.
    Inspect {
        /// RPM SPEC file to inspect.
        #[arg(value_name = "SPEC")]
        spec: PathBuf,
        /// Selects human or JSON output.
        #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
        format: ReportFormat,
    },
    /// Generates an openRuyi SPEC from a package manifest.
    #[command(
        after_help = "The default output is NAME.spec beside the manifest. Its parent directory must exist.\n\
For different existing content, select an output option or use the terminal menu.\n\
Without a usable terminal or an explicit action, conflicting output is an error.\n\
Sources with missing digests are downloaded automatically by the built-in HTTP client.\n\
Failures warn and leave that digest missing; use --offline to disable downloads."
    )]
    Gen {
        /// Package to generate.
        #[arg(value_name = "NAME")]
        name: String,
        /// Manifest to read; defaults to NAME.toml in the current directory.
        #[arg(long, value_name = "PATH")]
        manifest: Option<PathBuf>,
        /// Skip automatic downloads for missing Source SHA-256 digests.
        /// Missing digests remain warnings; existing digests and the TOML are never changed.
        #[arg(long)]
        offline: bool,
        /// Checks generation without writing SPEC files or consulting output conflicts.
        #[arg(long, conflicts_with_all = ["path", "stdout", "diff", "force", "skip_existing"])]
        check: bool,
        /// Selects the generation check report format.
        // Conflicts can waive `requires`, so reject non-check modes on this option too.
        #[arg(long, value_enum, requires = "check", conflicts_with_all = ["path", "stdout", "diff", "force", "skip_existing"])]
        format: Option<ReportFormat>,
        #[command(flatten, next_help_heading = "Output options")]
        output: output_cli::OutputOptions,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn report_format_does_not_mix_with_payload_outputs() {
        let edit_modes: &[&[&str]] = &[
            &["--view"],
            &["--schema"],
            &["--diff"],
            &["--stdout"],
            &["--editor", "vim"],
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
                if command == "gen" {
                    assert_eq!(
                        parse(&["--format", format]).err().map(|error| error.kind()),
                        Some(ErrorKind::MissingRequiredArgument)
                    );
                } else {
                    for action in [
                        vec!["--prepare", "drafts"],
                        vec!["--set", "package.version=2"],
                        vec!["--output", "other.spec", "--force"],
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
            ["--stdout", "--output=other.spec"],
            ["--check", "--stdout"],
            ["--check", "--diff"],
            ["--check", "--force"],
            ["--check", "--skip-existing"],
            ["--check", "--output=other.spec"],
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
