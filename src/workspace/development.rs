// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! A development area's package binding and cooperative operation lock.

use super::{Workspace, checkout, directory, invalid};
use crate::{check::metadata::Field, file_lock::FileLock};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// The selected user-owned input; derived SPEC artifacts never select themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GenerationInput {
    Authoring,
    Edit,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    pkg: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<GenerationInput>,
}

pub(crate) struct Development {
    path: PathBuf,
    binding: Binding,
    binding_contents: Option<String>,
    package_directory: PathBuf,
    plan: Option<checkout::Plan>,
    // This dedicated inode stays stable when the binding is atomically replaced.
    lock: Option<FileLock>,
}

impl Workspace {
    pub(crate) fn existing_development(&self, work: &str) -> io::Result<Development> {
        Field::Name
            .validate_at(work, "development area name")
            .map_err(invalid)?;
        let path = directory(&self.work, Path::new(work), false)?;
        fs::symlink_metadata(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "{}: development area does not exist: {error}",
                    path.display()
                ),
            )
        })?;
        // Retained build consumers need only the binding, not a reachable recipe repository.
        let (binding, lock, contents) = read_binding(&path, None, false)?;
        let checkout_path = path.join("checkout");
        let package_directory = checkout_path.join(&self.specs).join(&binding.pkg);
        Ok(Development {
            path,
            binding,
            binding_contents: Some(contents),
            package_directory,
            plan: None,
            lock,
        })
    }

    /// Materialize a bound WORK while retaining its existing operation lock.
    pub(crate) fn materialize(&self, development: &mut Development) -> io::Result<()> {
        let work_root = fs::canonicalize(&self.work)?;
        if development.path.parent() != Some(work_root.as_path())
            || development.package_directory
                != development
                    .path
                    .join("checkout")
                    .join(&self.specs)
                    .join(&development.binding.pkg)
        {
            return Err(invalid("development area belongs to a different workspace"));
        }
        development.verify_binding()?;
        let checkout_path = development.path.join("checkout");
        let work = development
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("development area has no UTF-8 name"))?;
        development.plan = checkout::prepare(
            &self.recipes,
            &checkout_path,
            &self.specs,
            development.package(),
            work,
        )?;
        development.create()?;
        development.verify_binding()
    }

    pub(crate) fn development(
        &self,
        work: &str,
        package: Option<&str>,
        preview: bool,
    ) -> io::Result<Development> {
        Field::Name
            .validate_at(work, "development area name")
            .map_err(invalid)?;
        if let Some(package) = package {
            Field::Name.validate(package).map_err(invalid)?;
        }
        let path = directory(&self.work, Path::new(work), false)?;
        let checkout_path = path.join("checkout");
        let existing = match fs::symlink_metadata(&path) {
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        let (binding, binding_contents, lock) = if existing {
            let (binding, lock, contents) = read_binding(&path, package, preview)?;
            (binding, Some(contents), lock)
        } else {
            (
                Binding {
                    pkg: package.unwrap_or(work).to_owned(),
                    input: None,
                },
                None,
                None,
            )
        };
        let plan = checkout::prepare(
            &self.recipes,
            &checkout_path,
            &self.specs,
            &binding.pkg,
            work,
        )?;
        if !existing
            && package.is_none()
            && let Some(plan) = &plan
        {
            plan.spec_name()?;
        }
        let manifest = path.join(format!("{}.toml", binding.pkg));
        match fs::symlink_metadata(&manifest) {
            Ok(metadata) if !metadata.is_file() => {
                return Err(invalid(format!(
                    "{} must be a regular authoring file, not a symlink",
                    manifest.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let package_directory = checkout_path.join(&self.specs).join(&binding.pkg);
        Ok(Development {
            path,
            binding,
            binding_contents,
            package_directory,
            plan,
            lock,
        })
    }
}

impl Development {
    pub(crate) fn directory(&self) -> &Path {
        &self.path
    }

    pub(crate) fn package_directory(&self) -> &Path {
        &self.package_directory
    }

    /// Resolve from the pinned commit before first allocation, or from the current checkout.
    pub(crate) fn spec(&self) -> io::Result<PathBuf> {
        if let Some(plan) = &self.plan {
            return Ok(self.package_directory.join(plan.spec_name()?));
        }
        let path = self
            .package_directory
            .join(format!("{}.spec", self.binding.pkg));
        if !fs::symlink_metadata(&path)?.is_file() {
            return Err(invalid(format!(
                "{} must be a regular SPEC, not a symlink",
                path.display()
            )));
        }
        Ok(path)
    }

    pub(crate) fn source(&self) -> io::Result<(PathBuf, String, Option<String>)> {
        if let Some(plan) = &self.plan {
            let (path, source, revision) = plan.source()?;
            Ok((path, source, Some(revision)))
        } else {
            let path = self.spec()?;
            let source = crate::utf8_file::read(&path).map_err(io::Error::other)?;
            Ok((path, source, None))
        }
    }

    pub(crate) fn package(&self) -> &str {
        &self.binding.pkg
    }

    pub(crate) fn manifest(&self) -> PathBuf {
        self.path.join(format!("{}.toml", self.binding.pkg))
    }

    /// A WORK with no completed input selection defaults to authoring, never a stage scan.
    pub(crate) fn generation_input(&self) -> GenerationInput {
        self.binding.input.unwrap_or(GenerationInput::Authoring)
    }

    /// An explicit SPEC destination cannot replace either WORK input or binding state.
    pub(crate) fn protect_output(&self, output: &Path) -> io::Result<()> {
        let target = crate::file_output::output_path(output).ok();
        for protected in [
            self.manifest(),
            self.path
                .join("stage")
                .join(format!("{}.toml", self.package())),
            self.path.join(".config.toml"),
            self.path.join(".lock"),
        ] {
            if target.as_ref() == Some(&protected)
                || crate::file_output::aliases(output, &protected)
            {
                return Err(invalid(format!(
                    "SPEC output {} must not replace WORK input or binding state",
                    output.display()
                )));
            }
        }
        Ok(())
    }

    /// Select an input only after the caller has completed the requested operation.
    pub(crate) fn select_input(&mut self, input: GenerationInput) -> io::Result<()> {
        self.verify_binding()?;
        if self.binding.input == Some(input) {
            return Ok(());
        }
        let mut binding = self.binding.clone();
        binding.input = Some(input);
        let contents = toml::to_string(&binding).map_err(io::Error::other)?;
        crate::file_output::write_artifact(&self.path.join(".config.toml"), contents.as_bytes())?;
        self.binding = binding;
        self.binding_contents = Some(contents);
        Ok(())
    }

    fn verify_binding(&self) -> io::Result<()> {
        let lock = self.lock.as_ref().ok_or_else(|| {
            invalid("cannot update a development area without the operation lock")
        })?;
        verify_file(&self.path.join(".lock"), lock.file())?;
        let config = self.path.join(".config.toml");
        let current = read_config(&config)?;
        if self.binding_contents.as_deref() != Some(current.as_str()) {
            return Err(invalid(format!(
                "{} changed during the operation; run the command again",
                config.display()
            )));
        }
        Ok(())
    }

    pub(crate) fn author_directory(&self) -> PathBuf {
        self.plan.as_ref().map_or_else(
            || self.path.join("checkout"),
            |plan| plan.repository().to_owned(),
        )
    }

    /// Publish only the package binding. Read-only operations stop here.
    pub(crate) fn ensure_work(&mut self) -> io::Result<()> {
        if self.lock.is_some() {
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| invalid("development area has no parent"))?;
        fs::create_dir_all(parent)?;
        fs::create_dir(&self.path)?;
        let lock = open_lock(&self.path)?;
        let text = toml::to_string(&self.binding).map_err(io::Error::other)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.path)?;
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist_noclobber(self.path.join(".config.toml"))
            .map_err(|error| error.error)?;
        self.binding_contents = Some(text);
        self.lock = Some(lock);
        Ok(())
    }

    /// Materialize the editable checkout without resetting an existing branch.
    pub(crate) fn create(&mut self) -> io::Result<()> {
        self.ensure_work()?;
        if let Some(plan) = &self.plan {
            plan.create()?;
            self.plan = None;
        }
        Ok(())
    }
}

fn read_binding(
    path: &Path,
    package: Option<&str>,
    preview: bool,
) -> io::Result<(Binding, Option<FileLock>, String)> {
    let config = path.join(".config.toml");
    regular_file(&config)?;
    let lock = if preview {
        None
    } else {
        Some(open_lock(path)?)
    };
    let text = read_config(&config)?;
    let binding: Binding =
        toml::from_str(&text).map_err(|error| invalid(format!("{}: {error}", config.display())))?;
    Field::Name.validate(&binding.pkg).map_err(invalid)?;
    if package.is_some_and(|package| package != binding.pkg) {
        return Err(invalid(format!(
            "{} is bound to package {}; --pkgname cannot change an existing binding",
            path.display(),
            binding.pkg
        )));
    }
    Ok((binding, lock, text))
}

fn read_config(config: &Path) -> io::Result<String> {
    let mut file = fs::File::open(config)?.into_file();
    verify_file(config, &file)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

fn open_lock(path: &Path) -> io::Result<FileLock> {
    let lock_path = path.join(".lock");
    let file = loop {
        match regular_file(&lock_path) {
            Ok(_) => {
                break fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&lock_path)?
                    .into_file();
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)
                {
                    Ok(file) => break file.into_file(),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    };
    verify_file(&lock_path, &file)?;
    let lock = FileLock::try_lock(file).map_err(|error| {
        io::Error::other(format!(
            "cannot lock development area {}: {error}; another operation may be running",
            path.display()
        ))
    })?;
    verify_file(&lock_path, lock.file())?;
    Ok(lock)
}

fn regular_file(path: &Path) -> io::Result<std::fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(invalid(format!(
            "{} must be a regular file, not a symlink",
            path.display(),
        )));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(invalid(format!(
            "{} must not have multiple hard links",
            path.display()
        )));
    }
    Ok(metadata)
}

fn verify_file(path: &Path, file: &File) -> io::Result<()> {
    let metadata = regular_file(path)?;
    #[cfg(unix)]
    {
        let opened = file.metadata()?;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
            return Err(invalid(format!(
                "{} was replaced during the operation",
                path.display()
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = (metadata, file);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(contents: &str) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(".config.toml"), contents).unwrap();
        directory
    }

    fn open(path: &Path, preview: bool) -> Development {
        let (binding, lock, contents) = read_binding(path, None, preview).unwrap();
        Development {
            path: path.to_path_buf(),
            binding,
            binding_contents: Some(contents),
            package_directory: path.join("checkout/SPECS/ed"),
            plan: None,
            lock,
        }
    }

    #[test]
    fn unselected_binding_defaults_to_authoring_without_inspecting_stage() {
        let directory = fixture("pkg = 'ed'\n");
        fs::create_dir_all(directory.path().join("stage/.state")).unwrap();
        fs::write(
            directory.path().join("stage/.state/index.toml"),
            "not a valid index",
        )
        .unwrap();
        let development = open(directory.path(), false);
        assert_eq!(development.generation_input(), GenerationInput::Authoring);
    }

    #[test]
    fn preview_reads_without_creating_a_lock_or_selecting_an_input() {
        let directory = fixture("pkg = 'ed'\ninput = 'edit'\n");
        let mut development = open(directory.path(), true);
        assert_eq!(development.generation_input(), GenerationInput::Edit);
        assert!(
            development
                .select_input(GenerationInput::Authoring)
                .is_err()
        );
        assert!(!directory.path().join(".lock").exists());
        assert_eq!(
            fs::read_to_string(directory.path().join(".config.toml")).unwrap(),
            "pkg = 'ed'\ninput = 'edit'\n"
        );
    }

    #[test]
    fn atomic_selection_keeps_the_operation_lock_and_pending_draft() {
        let directory = fixture("pkg = 'ed'\n");
        let path = directory.path();
        fs::create_dir(path.join("stage")).unwrap();
        fs::write(path.join("stage/ed.toml"), "unfinished user draft\n").unwrap();
        let mut development = open(path, false);
        #[cfg(unix)]
        let identity = fs::metadata(path.join(".lock")).unwrap().ino();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_done = done.clone();
        let config = path.join(".config.toml");
        let reader = std::thread::spawn(move || {
            while !reader_done.load(std::sync::atomic::Ordering::Acquire) {
                let binding: Binding =
                    toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
                assert_eq!(binding.pkg, "ed");
            }
        });
        for input in [GenerationInput::Edit, GenerationInput::Authoring]
            .into_iter()
            .cycle()
            .take(32)
        {
            development.select_input(input).unwrap();
            assert_eq!(development.generation_input(), input);
            let error = read_binding(path, None, false).err().unwrap();
            assert!(
                error.to_string().contains("cannot lock development area"),
                "{error}"
            );
            #[cfg(unix)]
            assert_eq!(fs::metadata(path.join(".lock")).unwrap().ino(), identity);
        }
        done.store(true, std::sync::atomic::Ordering::Release);
        reader.join().unwrap();
        assert_eq!(
            fs::read_to_string(path.join("stage/ed.toml")).unwrap(),
            "unfinished user draft\n"
        );
        #[cfg(unix)]
        let inherited = development
            .lock
            .as_ref()
            .unwrap()
            .file()
            .try_clone()
            .unwrap();
        drop(development);
        assert_eq!(
            open(path, false).generation_input(),
            GenerationInput::Authoring
        );
        #[cfg(unix)]
        drop(inherited);
    }

    #[test]
    fn external_binding_edits_are_not_lost_during_selection() {
        let directory = fixture("pkg = 'ed'\n");
        let mut development = open(directory.path(), false);
        let config = directory.path().join(".config.toml");
        fs::write(&config, "pkg = 'ed'\ninput = 'edit'\n# user note\n").unwrap();
        let error = development
            .select_input(GenerationInput::Authoring)
            .unwrap_err();
        assert!(
            error.to_string().contains("changed during the operation"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(config).unwrap(),
            "pkg = 'ed'\ninput = 'edit'\n# user note\n"
        );
    }

    #[test]
    fn materialization_retains_the_lock_and_original_binding() {
        use std::process::Command;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let recipes = root.join("openruyi");
        fs::create_dir_all(recipes.join("SPECS/ed")).unwrap();
        fs::write(recipes.join("SPECS/ed/ed.spec"), "recipe baseline\n").unwrap();
        for args in [
            vec!["init", "--quiet", "--initial-branch=main"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "Fixture recipe baseline"],
        ] {
            let output = Command::new("git")
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap())
                .env("HOME", root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Fixture Author")
                .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
                .env("GIT_COMMITTER_NAME", "Fixture Author")
                .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
                .current_dir(&recipes)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgSign=false",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
        let workspace = Workspace {
            root: root.to_path_buf(),
            recipes,
            work: root.join("work"),
            specs: PathBuf::from("SPECS"),
        };
        let area = workspace.work.join("review");
        fs::create_dir_all(&area).unwrap();
        let contents = "pkg = 'ed'\ninput = 'authoring'\n# user binding note\n";
        fs::write(area.join(".config.toml"), contents).unwrap();
        let mut development = workspace.existing_development("review").unwrap();
        #[cfg(unix)]
        let identity = fs::metadata(area.join(".lock")).unwrap().ino();
        assert!(!area.join("checkout").exists());
        workspace.materialize(&mut development).unwrap();
        assert_eq!(
            development.author_directory(),
            fs::canonicalize(area.join("checkout")).unwrap()
        );
        assert_eq!(
            fs::read_to_string(development.spec().unwrap()).unwrap(),
            "recipe baseline\n"
        );
        // Existing checkout validation must also reuse, not reacquire, this lock.
        workspace.materialize(&mut development).unwrap();
        let error = workspace.existing_development("review").err().unwrap();
        assert!(
            error.to_string().contains("cannot lock development area"),
            "{error}"
        );
        #[cfg(unix)]
        assert_eq!(fs::metadata(area.join(".lock")).unwrap().ino(), identity);
        assert_eq!(
            fs::read_to_string(area.join(".config.toml")).unwrap(),
            contents
        );
        assert_eq!(development.generation_input(), GenerationInput::Authoring);
        drop(development);
        assert!(workspace.existing_development("review").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn lock_aliases_and_replaced_lock_inodes_are_rejected() {
        use std::os::unix::fs::symlink;

        for symbolic in [false, true] {
            let directory = fixture("pkg = 'ed'\n");
            let target = directory.path().join("keep");
            fs::write(&target, "user bytes\n").unwrap();
            let lock_path = directory.path().join(".lock");
            if symbolic {
                symlink(&target, &lock_path).unwrap();
            } else {
                fs::hard_link(&target, &lock_path).unwrap();
            }
            assert!(read_binding(directory.path(), None, false).is_err());
            assert_eq!(fs::read_to_string(target).unwrap(), "user bytes\n");
        }
        let directory = fixture("pkg = 'ed'\n");
        let mut development = open(directory.path(), false);
        let lock_path = directory.path().join(".lock");
        fs::rename(&lock_path, directory.path().join("old.lock")).unwrap();
        fs::write(&lock_path, "").unwrap();
        let error = development.select_input(GenerationInput::Edit).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("was replaced during the operation"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join(".config.toml")).unwrap(),
            "pkg = 'ed'\n"
        );
    }
}
