// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Rotate attempt evidence without deleting a developer's retained environment.

use super::{InputFile, directory, invalid, regular_file};
use fs_err as fs;
use serde_json::Value;
use std::{
    io,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) struct Previous {
    path: PathBuf,
    pub(super) receipt: Value,
    _lock: crate::file_lock::FileLock,
    interactive: bool,
}

pub(super) fn archive(output: &Path, package: &str) -> io::Result<Option<Previous>> {
    if !output.try_exists()? {
        return Ok(None);
    }
    directory(output)?;
    let receipt = output.join("receipt.json");
    regular_file(&receipt)?;
    let lock = crate::file_lock::FileLock::try_lock(fs::File::open(&receipt)?.into_file())
        .map_err(|e| invalid(format!("build is in use by another operation: {e}")))?;
    let mut bytes = Vec::new();
    lock.file().read_to_end(&mut bytes)?;
    let receipt: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if receipt["format_version"] != 1
        || receipt["backend"] != "compose"
        || receipt["engine"] != "mock"
        || receipt["package"] != package
    {
        return Err(invalid(
            "existing result is not a supported build receipt; nothing moved",
        ));
    }
    let interactive = session_used(output)?;
    let history = output.parent().expect("build parent").join("build-history");
    fs::create_dir_all(&history)?;
    directory(&history)?;
    let path = history.join(super::compose::operation_id("attempt"));
    fs::rename(output, &path)?;
    eprintln!(
        "build: previous result {}{}",
        crate::output_cli::human_path(&path).display(),
        if interactive || receipt["session_tracking"] != true {
            "; environment retained (session used or state unknown)"
        } else {
            ""
        }
    );
    Ok(Some(Previous {
        path,
        receipt,
        _lock: lock,
        interactive,
    }))
}

impl Previous {
    pub(super) fn reusable(&self, config: &[InputFile]) -> bool {
        !self.interactive
            && self.receipt["session_tracking"] == true
            && self.receipt["resources_retained"] != false
            && self.receipt["remove_requested"] == false
            && self.receipt["execution"]["cleanup_failure"].is_null()
            && self.receipt["execution"]["artifact_error"].is_null()
            && self.receipt["engine_validation_error"].is_null()
            && self.receipt["execution"]["details"]["container_id"].is_string()
            && self.receipt["configuration_files"]
                == serde_json::to_value(config).expect("file metadata")
    }

    pub(super) fn transfer(&self, output: &Path) -> io::Result<crate::file_lock::FileLock> {
        // Save a recovery receipt before the old attempt relinquishes ownership.
        // Historical commands/details remain evidence, not cleanup authority.
        let mut recovery = self.receipt.clone();
        recovery["success"] = false.into();
        recovery["execution"]["failure"] =
            "new build started; recover retained environment or retry".into();
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(output.join("receipt.json"))?
            .into_file();
        let lock = crate::file_lock::FileLock::try_lock(file)?;
        serde_json::to_writer_pretty(lock.file(), &recovery).map_err(io::Error::other)?;
        lock.file().sync_all()?;
        let mut historical = self.receipt.clone();
        historical["resources_retained"] = false.into();
        crate::file_output::write_artifact(
            &self.path.join("receipt.json"),
            &serde_json::to_vec_pretty(&historical).map_err(io::Error::other)?,
        )?;
        eprintln!("build: reusing worker and dependency caches; building current recipe inputs");
        Ok(lock)
    }
}

/// One operation record, independent of log filenames. Unknown/corrupt records block reuse.
pub(crate) fn session_used(output: &Path) -> io::Result<bool> {
    let path = output.join("session.toml");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
        Ok(_) => {
            regular_file(&path)?;
            let value: toml::Table =
                toml::from_str(&fs::read_to_string(path)?).map_err(io::Error::other)?;
            value
                .get("environment_may_have_changed")
                .and_then(toml::Value::as_bool)
                .ok_or_else(|| invalid("invalid shell session record"))
        }
    }
}

pub(crate) fn select(root: &Path, attempt: &str) -> io::Result<PathBuf> {
    if attempt.is_empty()
        || !attempt
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(invalid(
            "build ID must be current or a single listed identifier",
        ));
    }
    let path = if attempt == "current" {
        root.join("build")
    } else {
        directory(&root.join("build-history"))?.join(attempt)
    };
    directory(&path)
}

#[derive(serde::Serialize)]
pub(crate) struct Attempt {
    pub id: String,
    pub directory: PathBuf,
    pub success: Option<bool>,
    pub resources_retained: Option<bool>,
    pub environment_may_have_changed: Option<bool>,
    pub error: Option<String>,
    pub target_arch: Option<String>,
    pub daemon_arch: Option<String>,
}

pub(crate) fn list(root: &Path) -> io::Result<Vec<Attempt>> {
    let mut ids = Vec::new();
    if root.join("build").try_exists()? {
        ids.push("current".to_owned());
    }
    let history = root.join("build-history");
    if history.try_exists()? {
        directory(&history)?;
        for entry in fs::read_dir(history)? {
            let entry = entry?;
            ids.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid("build ID is not UTF-8"))?,
            );
        }
    }
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let directory = select(root, &id)?;
            let mut attempt = Attempt {
                id,
                directory,
                success: None,
                resources_retained: None,
                environment_may_have_changed: None,
                error: None,
                target_arch: None,
                daemon_arch: None,
            };
            let read = || -> io::Result<(Value, bool)> {
                let path = attempt.directory.join("receipt.json");
                regular_file(&path)?;
                let receipt = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
                Ok((receipt, session_used(&attempt.directory)?))
            };
            match read() {
                Ok((receipt, session)) => {
                    let environment = &receipt["execution"]["details"]["environment"];
                    attempt.target_arch = environment["probe"]["target_arch"]
                        .as_str()
                        .map(str::to_owned);
                    attempt.daemon_arch = environment["daemon_arch"].as_str().map(str::to_owned);
                    attempt.success = receipt["success"].as_bool();
                    attempt.resources_retained = receipt["resources_retained"].as_bool();
                    attempt.environment_may_have_changed =
                        (receipt["session_tracking"] == true).then_some(session);
                }
                Err(error) => attempt.error = Some(error.to_string()),
            }
            Ok(attempt)
        })
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn attempt_selection_stays_inside_the_work_area() {
        let work = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(outside.path().join("attempt-example")).unwrap();
        std::os::unix::fs::symlink(outside.path(), work.path().join("build-history")).unwrap();
        assert!(super::select(work.path(), "attempt-example").is_err());
        assert!(super::select(work.path(), "../outside").is_err());
        std::fs::remove_file(work.path().join("build-history")).unwrap();
        std::fs::create_dir_all(work.path().join("build-history/attempt-example")).unwrap();
        assert!(
            super::select(work.path(), "attempt-example")
                .unwrap()
                .starts_with(work.path().canonicalize().unwrap())
        );
    }
}

#[derive(clap::Args)]
pub(crate) struct Selection {
    /// Existing development area.
    #[arg(
        value_name = "WORK",
        required_unless_present = "build_dir",
        conflicts_with = "build_dir"
    )]
    pub(crate) work: Option<String>,
    /// Select one explicit receipt-bound result directory.
    #[arg(long, value_name = "PATH", conflicts_with = "work")]
    build_dir: Option<PathBuf>,
    /// Select current or a build ID listed by inspect WORK.
    #[arg(long, requires = "work", conflicts_with = "build_dir")]
    attempt: Option<String>,
}

pub(crate) struct Selected {
    pub(crate) development: Option<crate::workspace::Development>,
    pub(crate) path: PathBuf,
}

impl Selection {
    pub(crate) fn resolve(&self) -> io::Result<Selected> {
        let development = self
            .work
            .as_deref()
            .map(|work| crate::workspace::discover()?.existing_development(work))
            .transpose()?;
        let path = if let Some(area) = &development {
            match self.attempt.as_deref() {
                Some(id) => select(area.directory(), id)?,
                None => area.directory().join("build"),
            }
        } else {
            self.build_dir
                .clone()
                .ok_or_else(|| invalid("select WORK or --build-dir PATH"))?
        };
        Ok(Selected { development, path })
    }
}
