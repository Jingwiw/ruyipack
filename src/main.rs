// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line entry point for `RuyiPack`.

mod check;
mod check_command;
mod check_report;
mod cli;
mod edit;
mod file_output;
mod generate;
mod inspect;
mod parser_diagnostic;
mod render;
mod utf8_file;

use std::{
    fmt,
    io::{self, Write},
    process::ExitCode,
};

use clap::Parser;
use cli::{ArtifactFormat, Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Edit(options) => exit_for(edit::run(&options)),
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
