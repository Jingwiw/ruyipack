// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line entry point for `RuyiPack`.

mod build;
mod check;
mod check_report;
mod clean;
mod cli;
mod edit;
mod environment;
mod file_digest;
mod file_output;
mod generate;
mod host_process;
mod inspect;
mod new;
mod output_cli;
mod parser_diagnostic;
mod profile;
mod render;
mod report;
mod schema;
mod source;
mod source_hash;
mod source_location;
mod spec;
mod spec_metadata;
mod stage;
mod tool;
mod utf8_file;
mod verify_sources;
mod workspace;

use std::{fmt, process::ExitCode};

use clap::Parser;
use cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Init(options) => exit_for(workspace::run(&options).map(|()| true)),
        Command::Build(options) => exit_for(build::run(&options)),
        Command::Shell(options) => exit_for(build::shell::run(&options)),
        Command::Schema(options) => exit_for(schema::run(&options)),
        Command::Clean(options) => exit_for(clean::run(&options)),
        Command::New(options) => exit_for(new::run(&options).map(|()| true)),
        Command::Edit(options) => exit_for(edit::run(options)),
        Command::Source(command) => match command {
            cli::SourceCommand::Hash(options) => exit_for(source_hash::run(&options)),
            cli::SourceCommand::Verify(options) => exit_for(verify_sources::run(&options)),
        },
        Command::Check(options) => exit_for(check::run(&options)),
        Command::Inspect(options) => exit_for(inspect::run(&options)),
        Command::Gen(options) => exit_for(generate::run(&options)),
    }
}

/// Maps a command result to the shared CLI exit contract.
fn exit_for<E: fmt::Display>(result: Result<bool, E>) -> ExitCode {
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            // The command still fails when stderr is unavailable.
            let _ = output_cli::stderr().message(
                output_cli::HumanLevel::Error,
                None,
                format_args!("{error}"),
            );
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
