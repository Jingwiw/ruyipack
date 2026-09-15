// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line entry point for `RuyiPack`.

mod check;
mod check_command;
mod check_report;
mod file_output;
mod generate;
mod inspect;
mod parser_diagnostic;
mod render;
mod utf8_file;

use std::{
    fmt,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

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
    /// Generates an artifact from a `RuyiPack` manifest.
    #[command(
        after_help = "The default output is NAME.spec beside the manifest. Its parent directory must exist.\n\
For different existing content, select an output option or use the terminal menu.\n\
Without a usable terminal or an explicit action, conflicting output is an error."
    )]
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
        Command::Check { spec, format } => exit_for(check_command::run(&spec, format)),
        Command::Inspect { spec, format } => exit_for(inspect::run(&spec, format).map(|()| true)),
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
            // The command still fails when stderr is unavailable.
            let _ = writeln!(io::stderr().lock(), "error: {error}");
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
