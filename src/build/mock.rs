// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

use super::Engine;
use serde::Deserialize;
use std::path::{Component, PathBuf};

#[derive(Deserialize)]
struct Receipt {
    format_version: u32,
    engine: String,
    success: bool,
    target_stage: super::Stage,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    path: PathBuf,
    #[serde(flatten)]
    content: crate::file_digest::Content,
}

use std::{io, path::Path, time::Duration};

pub(super) struct Mock(pub(super) super::Stage);

impl Engine for Mock {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn shell(&self) -> Vec<String> {
        python(include_str!("mock_shell.py"))
    }

    fn export_patch(&self) -> Vec<String> {
        python(include_str!("mock_export.py"))
    }

    fn stage(
        &self,
        input_dir: &Path,
        spec_name: &str,
        timeout: Duration,
    ) -> io::Result<Vec<String>> {
        fs_err::write(input_dir.join("engine.py"), include_str!("mock.py"))?;
        Ok(vec![
            "python3".into(),
            "/input/engine.py".into(),
            "--spec".into(),
            format!("/input/SPECS/{spec_name}"),
            "--sources".into(),
            "/input/SOURCES".into(),
            "--output".into(),
            "/output".into(),
            "--timeout".into(),
            timeout.as_secs().to_string(),
            "--stage".into(),
            match self.0 {
                super::Stage::Prep => "prep",
                super::Stage::Build => "build",
            }
            .into(),
        ])
    }
    fn verify_result(&self, output: &Path) -> io::Result<()> {
        let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
        let receipt_path = output.join("receipt.json");
        if !fs_err::symlink_metadata(&receipt_path)?.is_file() {
            return Err(invalid("engine receipt must be a regular file"));
        }
        let receipt: Receipt =
            serde_json::from_slice(&fs_err::read(receipt_path)?).map_err(io::Error::other)?;
        if receipt.format_version != 1
            || receipt.engine != self.name()
            || receipt.target_stage != self.0
            || !receipt.success
            || receipt.artifacts.is_empty()
        {
            return Err(invalid(
                "Mock did not return a successful build receipt with artifacts",
            ));
        }
        for artifact in receipt.artifacts {
            if artifact.path.as_os_str().is_empty() {
                return Err(invalid("empty artifact path"));
            }
            let mut path = output.to_path_buf();
            for part in artifact.path.components() {
                if !matches!(part, Component::Normal(_)) {
                    return Err(invalid(
                        "artifact path must stay inside the output directory",
                    ));
                }
                path.push(part);
                if fs_err::symlink_metadata(&path)?.is_symlink() {
                    return Err(invalid("artifact path must not contain symlinks"));
                }
            }
            if crate::file_digest::read(&path).map_err(io::Error::other)? != artifact.content {
                return Err(invalid("artifact bytes do not match the engine receipt"));
            }
        }
        Ok(())
    }
}

fn python(script: &str) -> Vec<String> {
    vec!["python3".into(), "-c".into(), script.into()]
}
