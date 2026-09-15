// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line entry point for `RuyiPack`.

mod check;
mod check_report;
mod file_output;
mod generate;
mod inspect;
mod parser_diagnostic;
mod render;
mod utf8_file;

use std::{fmt, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Checks required main-package tag presence in an RPM SPEC file.
    Check {
        /// RPM SPEC file to check.
        #[arg(value_name = "SPEC")]
        spec: PathBuf,
        /// Selects human or JSON output.
        #[arg(long, value_enum, default_value_t = check::CheckFormat::Human)]
        format: check::CheckFormat,
    },
    /// Prints the normalized main-package tags from an RPM SPEC file.
    Inspect {
        /// RPM SPEC file to inspect.
        #[arg(value_name = "SPEC")]
        spec: PathBuf,
    },
    /// Generates an artifact from a `RuyiPack` manifest.
    Gen {
        /// Package to generate.
        #[arg(value_name = "NAME")]
        name: String,
        /// Artifact format to generate.
        #[arg(long, value_enum, default_value = "spec")]
        format: ArtifactFormat,
        /// Manifest to read; defaults to NAME.toml in the current directory.
        #[arg(long, value_name = "PATH")]
        manifest: Option<PathBuf>,
        #[command(flatten, next_help_heading = "Output options")]
        output: file_output::OutputOptions,
    },
}

#[derive(Clone, ValueEnum)]
enum ArtifactFormat {
    Spec,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { spec, format } => exit_for(check::run(&spec, format)),
        Command::Inspect { spec } => exit_for(inspect::run(&spec).map(|()| true)),
        Command::Gen {
            name,
            format: ArtifactFormat::Spec,
            manifest,
            output,
        } => exit_for(generate::run(&name, manifest.as_deref(), &output).map(|()| true)),
    }
}

/// Maps a command result to the shared CLI exit contract.
fn exit_for<E: fmt::Display>(result: Result<bool, E>) -> ExitCode {
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn command_definition_is_consistent() {
        super::Cli::command().debug_assert();
    }
}
