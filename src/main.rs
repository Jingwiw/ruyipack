// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line entry point for `RuyiPack`.

mod check;
mod check_report;
mod cli;
mod edit;
mod file_output;
mod generate;
mod init;
mod inspect;
mod output_cli;
mod parser_diagnostic;
mod profile;
mod render;
mod source;
mod source_hash;
mod source_location;
mod spec;
mod spec_metadata;
mod tool;
mod utf8_file;
mod verify_sources;

use std::{
    fmt,
    io::{self, Write},
    process::ExitCode,
};

use clap::Parser;
use cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Init(options) => exit_for(init::run(&options).map(|()| true)),
        Command::Edit(options) => exit_for(edit::run(options)),
        Command::SourceHash(options) => exit_for(source_hash::run(&options)),
        Command::VerifySources(options) => exit_for(verify_sources::run(&options)),
        Command::Check { spec, format } => exit_for(check::run(&spec, format)),
        Command::Inspect { spec, format } => exit_for(inspect::run(&spec, format)),
        Command::Gen {
            name,
            manifest,
            offline,
            check,
            format,
            output,
        } => exit_for(generate::run(
            &name,
            manifest.as_deref(),
            offline,
            &output,
            check,
            format,
        )),
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
