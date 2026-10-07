// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Run the prepared directory’s `RemoteAsset` check in a clean environment.

mod evidence;
pub(crate) use evidence::verify;

use crate::{
    environment::{Backend, compose::Compose},
    output_cli::{ReportError, ReportFormat},
};
use fs_err as fs;
use serde::Serialize;
use std::{io, path::PathBuf, time::Duration};

#[derive(clap::Args)]
#[group(id = "directory-options", multiple = true)]
pub(crate) struct Options {
    /// Run scripts/remoteassetify.py for SPECs in this prepared directory.
    #[arg(long, value_name="DIR", requires="check_output",
        conflicts_with_all=["work", "spec", "manifest", "pkgname", "auto_fix", "upgrade", "materials", "policy", "defines"])]
    pub(crate) directory: Option<PathBuf>,
    /// New directory for immutable inputs, logs and the check receipt.
    #[arg(long, requires = "directory", value_name = "DIR")]
    check_output: Option<PathBuf>,
    /// Override the check environment with a Compose file containing service worker.
    #[arg(long, requires = "directory", value_name = "FILE")]
    check_config: Option<PathBuf>,
    /// Docker connection for upstream checks; defaults to the current context.
    #[arg(long, requires = "directory")]
    context: Option<String>,
    /// Whole container execution budget in seconds; recovery has a separate bound.
    #[arg(long, requires="directory", default_value_t=300, value_parser=clap::value_parser!(u64).range(1..))]
    timeout: u64,
}

#[derive(Serialize)]
struct Receipt {
    format_version: u32,
    operation: &'static str,
    coverage: &'static str,
    directory: PathBuf,
    inputs: crate::workspace::baseline::Files,
    config: PathBuf,
    config_sha256: String,
    driver_sha256: String,
    tool: crate::tool::Identity,
    execution: crate::environment::Execution,
    checks: Vec<Check>,
    success: bool,
    result_error: Option<String>,
}

#[derive(serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Check {
    path: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn specs(inputs: &crate::workspace::baseline::Files) -> impl Iterator<Item = &str> {
    inputs
        .keys()
        .filter(|path| path.starts_with("SPECS/") && path.ends_with(".spec"))
        .map(String::as_str)
}

fn complete(inputs: &crate::workspace::baseline::Files, checks: &[Check]) -> bool {
    checks
        .iter()
        .map(|check| check.path.as_str())
        .eq(specs(inputs))
}

#[derive(serde::Deserialize)]
struct Results {
    checks: Vec<Check>,
}

pub(crate) fn run(options: &Options, format: ReportFormat) -> Result<bool, ReportError> {
    crate::host_process::install_handler()?;
    let directory = fs::canonicalize(options.directory.as_ref().expect("Clap input"))?;
    let output = options.check_output.as_ref().expect("Clap output");
    fs::create_dir(output)?;
    let output = fs::canonicalize(output)?;
    if output.starts_with(directory.join("SPECS")) {
        return Err(io::Error::other("check output must be outside SPECS").into());
    }
    let input = output.join("input");
    fs::create_dir(&input)?;
    // Freeze the supplied files, not another Git revision. Copy also retains local materials.
    if !fs::symlink_metadata(directory.join("SPECS"))?.is_dir() {
        return Err(io::Error::other("SPECS must be an ordinary directory").into());
    }
    crate::file_tree::copy(&directory.join("SPECS"), &input.join("SPECS"))?;
    let scripts = directory.join("scripts");
    if !fs::symlink_metadata(&scripts)?.is_dir() {
        return Err(io::Error::other("scripts must be an ordinary directory").into());
    }
    let source = scripts.join("remoteassetify.py");
    crate::file_digest::read(&source).map_err(io::Error::other)?;
    fs::create_dir(input.join("scripts"))?;
    fs::copy(source, input.join("scripts/remoteassetify.py"))?;
    let inputs = crate::workspace::baseline::read(&input)?;
    let spec_count = specs(&inputs).count();
    crate::report::write(
        &mut fs::File::create(input.join("selection.toml"))?,
        &inputs,
    )?;
    fs::write(input.join("run.py"), include_bytes!("runner.py"))?;
    let driver_sha256 = crate::file_digest::read(&input.join("run.py"))
        .map_err(io::Error::other)?
        .sha256;
    let config = if let Some(path) = &options.check_config {
        fs::canonicalize(path)?
    } else {
        let root = output.join("environment");
        fs::create_dir(&root)?;
        fs::write(
            root.join("Dockerfile"),
            include_bytes!("../../environments/check/Dockerfile"),
        )?;
        fs::write(
            root.join("compose.yaml"),
            include_bytes!("../../environments/check/compose.yaml"),
        )?;
        root.join("compose.yaml")
    };
    let config_sha256 = crate::file_digest::read(&config)
        .map_err(io::Error::other)?
        .sha256;
    let backend = Compose {
        context: options.context.as_deref(),
        config: &config,
        remove: true,
        probe: None,
    };
    backend.validate()?;
    let invocation = vec!["python3".into(), "/input/run.py".into()];
    let execution = if spec_count == 0 {
        crate::environment::Execution {
            success: true,
            ..Default::default()
        }
    } else {
        backend.execute(
            None,
            &invocation,
            &input,
            &output,
            Duration::from_secs(options.timeout),
        )
    };
    let result = if spec_count == 0 {
        Ok(Results { checks: Vec::new() })
    } else {
        fs::read_to_string(output.join("engine/results.toml"))
            .and_then(|text| toml::from_str::<Results>(&text).map_err(io::Error::other))
    };
    let (checks, mut result_error) = match result {
        Ok(results) => (results.checks, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    if result_error.is_none() && !complete(&inputs, &checks) {
        result_error = Some("check results do not match all selected candidate files".into());
    }
    if crate::file_digest::read(&config)
        .map(|content| content.sha256)
        .ok()
        .as_deref()
        != Some(&config_sha256)
    {
        result_error = Some("check configuration changed during execution".into());
    }
    let success =
        execution.success && result_error.is_none() && checks.iter().all(|c| c.exit_code == 0);
    let receipt = Receipt {
        format_version: 1,
        operation: "directory-check",
        coverage: "remoteasset: prepared directory SPECs; not full Action parity",
        directory,
        inputs,
        config,
        config_sha256,
        driver_sha256,
        tool: crate::tool::identity(),
        execution,
        checks,
        success,
        result_error,
    };
    crate::report::write(
        &mut fs::File::create(output.join("receipt.toml"))?,
        &receipt,
    )?;
    match format {
        ReportFormat::Toml => {
            crate::report::write(&mut io::stdout().lock(), &receipt)
                .map_err(ReportError::Stdout)?;
        }
        ReportFormat::Human => crate::output_cli::stderr().message(
            if success {
                crate::output_cli::HumanLevel::Info
            } else {
                crate::output_cli::HumanLevel::Error
            },
            None,
            format_args!(
                "RemoteAsset: {}/{} checks passed; receipt {}",
                receipt.checks.iter().filter(|c| c.exit_code == 0).count(),
                spec_count,
                crate::output_cli::human_path(&output.join("receipt.toml")).display()
            ),
        )?,
    }
    Ok(success)
}
