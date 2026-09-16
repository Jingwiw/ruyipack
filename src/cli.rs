// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Command-line syntax and help.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::{check_command, file_output, inspect};

#[derive(Parser)]
#[command(version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
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
pub(crate) enum ArtifactFormat {
    Spec,
}
