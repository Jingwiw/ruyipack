// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Remove receipt-owned build resources, never a development checkout or authoring input.

use std::{io, path::PathBuf, time::Duration};

use clap::Args;

use crate::output_cli::ReportFormat;

#[derive(Args)]
pub(crate) struct Options {
    /// Existing workspace development area whose retained build will be removed.
    #[arg(
        value_name = "WORK",
        required_unless_present = "build_dir",
        conflicts_with = "build_dir"
    )]
    work: Option<String>,
    /// Advanced: remove one explicit receipt-bound build directory.
    #[arg(long, value_name = "PATH", conflicts_with = "work")]
    build_dir: Option<PathBuf>,
    /// Skip confirmation; resource ownership, daemon identity and locks still apply.
    #[arg(long)]
    force: bool,
    /// Override Docker's current connection; the recorded daemon must still match.
    #[arg(long)]
    context: Option<String>,
    /// Whole cleanup execution deadline.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    format: ReportFormat,
}

pub(crate) fn run(options: &Options) -> io::Result<bool> {
    // Use the same lock ordering as shell: WORK binding first, then the build receipt.
    let development = options
        .work
        .as_deref()
        .map(|work| crate::workspace::discover()?.existing_development(work))
        .transpose()?;
    let path = development.as_ref().map_or_else(
        || {
            options
                .build_dir
                .as_ref()
                .expect("clap requires WORK or --build-dir")
                .clone()
        },
        |area| area.directory().join("build"),
    );
    crate::build::clean_result(
        &path,
        options.force,
        options.context.as_deref(),
        Duration::from_secs(options.timeout),
        options.format,
    )
}
