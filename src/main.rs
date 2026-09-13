// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line entry point for RuyiPack.

mod check;
mod check_report;
mod inspect;
mod parser_diagnostic;
mod utf8_file;

use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check { spec, format } => check::run(&spec, format),
        Command::Inspect { spec } => inspect::run(&spec).map(|()| true),
    };

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
