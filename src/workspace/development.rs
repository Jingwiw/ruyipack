// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! A development area's package binding and cooperative operation lock.

use super::{Workspace, directory, invalid, recipe};
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    pkg: String,
    #[serde(default)]
    kind: DevelopmentKind,
}

/// A WORK's fixed recipe and authoring layout, never inferred from cached files.
#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DevelopmentKind {
    Local,
    #[default]
    Repository,
    Spec,
}

pub(crate) struct Development {
    path: PathBuf,
    binding: Binding,
    binding_contents: Option<String>,
    package_directory: PathBuf,
    plan: Option<recipe::Plan>,
    // The dedicated inode serializes operations independently of configuration contents.
    lock: Option<FileLock>,
}

impl Workspace {
    fn package_directory(path: &Path, binding: &Binding) -> io::Result<PathBuf> {
        let relative = Path::new("recipe").join("SPECS").join(&binding.pkg);
        directory(path, &relative, false)
    }

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
        let package_directory = Self::package_directory(&path, &binding)?;
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
                != Self::package_directory(&development.path, &development.binding)?
        {
            return Err(invalid("development area belongs to a different workspace"));
        }
        development.verify_binding()?;
        if development.binding.kind == DevelopmentKind::Repository {
            development.plan = recipe::prepare(
                &self.recipes,
                &development.package_directory,
                &self.specs,
                development.package(),
            )?;
        }
        development.create()?;
        development.verify_binding()
    }

    pub(crate) fn development(
        &self,
        work: &str,
        package: Option<&str>,
        preview: bool,
    ) -> io::Result<Development> {
        self.resolve_development(work, package, preview, None)
    }

    pub(crate) fn new_development(
        &self,
        work: &str,
        package: Option<&str>,
        preview: bool,
        kind: DevelopmentKind,
    ) -> io::Result<Development> {
        self.resolve_development(work, package, preview, Some(kind))
    }

    fn resolve_development(
        &self,
        work: &str,
        package: Option<&str>,
        preview: bool,
        new: Option<DevelopmentKind>,
    ) -> io::Result<Development> {
        Field::Name
            .validate_at(work, "development area name")
            .map_err(invalid)?;
        if let Some(package) = package {
            Field::Name.validate(package).map_err(invalid)?;
        }
        let path = directory(&self.work, Path::new(work), false)?;

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
                    kind: new.unwrap_or_default(),
                },
                None,
                None,
            )
        };
        if new.is_some_and(|kind| kind != binding.kind) {
            return Err(invalid(
                "existing WORK has a different recipe or authoring kind; use another WORK",
            ));
        }
        let plan = if binding.kind == DevelopmentKind::Repository {
            recipe::prepare(
                &self.recipes,
                &Self::package_directory(&path, &binding)?,
                &self.specs,
                &binding.pkg,
            )?
        } else {
            None
        };
        if !existing
            && new.is_none()
            && package.is_none()
            && let Some(plan) = &plan
        {
            plan.spec_name()?;
        }
        let manifest = binding.manifest(&path);
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
        let package_directory = Self::package_directory(&path, &binding)?;
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

impl Binding {
    fn manifest(&self, root: &Path) -> PathBuf {
        if self.kind != DevelopmentKind::Local {
            let spec =
                super::recipe::spec_in(&root.join("recipe/SPECS").join(&self.pkg), Some(&self.pkg))
                    .unwrap_or_else(|_| PathBuf::from(format!("{}.spec", self.pkg)));
            return root.join(
                spec.with_extension("toml")
                    .file_name()
                    .expect("SPEC filename"),
            );
        }
        root.join(format!("{}.toml", self.pkg))
    }
}

impl Development {
    pub(crate) fn spec_authoring(&self) -> bool {
        self.binding.kind != DevelopmentKind::Local
    }

    pub(crate) fn editor_directory(&self) -> PathBuf {
        self.package_directory.clone()
    }

    pub(crate) fn sources(&self) -> PathBuf {
        self.directory().join("sources")
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.path
    }

    pub(crate) fn package_directory(&self) -> &Path {
        &self.package_directory
    }

    /// Resolve from the pinned commit before first allocation, or from the current recipe.
    pub(crate) fn spec(&self) -> io::Result<PathBuf> {
        if let Some(plan) = &self.plan {
            return Ok(self.package_directory.join(plan.spec_name()?));
        }
        super::recipe::spec_in(&self.package_directory, Some(self.package()))
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
        self.binding.manifest(&self.path)
    }

    /// An explicit SPEC destination cannot replace either WORK input or binding state.
    pub(crate) fn protect_output(&self, output: &Path) -> io::Result<()> {
        if self.spec_authoring() {
            crate::draft::protect_output(output, Some(&self.manifest())).map_err(invalid)?;
        }
        let target = crate::file_output::output_path(output).ok();
        for protected in [
            self.manifest(),
            self.path.join(".config.toml"),
            self.path.join(".lock"),
            self.path.join("baseline.toml"),
            self.path.join("commit.toml"),
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

    pub(super) fn verify_binding(&self) -> io::Result<()> {
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
        let text = self.write_binding(&self.path)?;
        self.binding_contents = Some(text);
        self.lock = Some(lock);
        Ok(())
    }

    fn write_binding(&self, directory: &Path) -> io::Result<String> {
        let text = toml::to_string(&self.binding).map_err(io::Error::other)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist_noclobber(directory.join(".config.toml"))
            .map_err(|error| error.error)?;
        Ok(text)
    }

    /// Publish a complete import without replacing even an empty competing WORK.
    pub(crate) fn publish_prepared(&mut self, directory: &Path) -> io::Result<()> {
        if self.lock.is_some() || self.binding.kind != DevelopmentKind::Spec {
            return Err(invalid("import requires a new SPEC development area"));
        }
        let lock = open_lock(directory)?;
        let text = self.write_binding(directory)?;
        publish_directory(directory, &self.path)?;
        self.binding_contents = Some(text);
        self.lock = Some(lock);
        Ok(())
    }

    /// Copy committed package files once; subsequent edits stay in the ordinary recipe directory.
    pub(crate) fn create(&mut self) -> io::Result<()> {
        self.ensure_work()?;
        if let Some(plan) = &self.plan {
            plan.create(&self.path)?;
            self.plan = None;
        } else {
            fs::create_dir_all(&self.package_directory)?;
            if !self.path.join("baseline.toml").exists() {
                super::baseline::save(
                    &self.path.join("baseline.toml"),
                    &super::baseline::Baseline {
                        files: super::baseline::Files::new(),
                        allow_create: true,
                    },
                )?;
            }
        }
        Ok(())
    }
}

pub(super) fn publish_directory(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE).map_err(|error| {
            io::Error::new(
                io::Error::from(error).kind(),
                format!("cannot publish WORK {}: {error}", target.display()),
            )
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (source, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic WORK import requires Linux or macOS",
        ))
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
            package_directory: path.join("recipe/SPECS/ed"),
            plan: None,
            lock,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn prepared_directory_never_replaces_an_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("prepared");
        let target = root.path().join("work");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("recipe"), "candidate").unwrap();
        fs::create_dir(&target).unwrap();
        assert!(publish_directory(&source, &target).is_err());
        assert!(target.read_dir().unwrap().next().is_none());
        fs::write(target.join("recipe"), "existing").unwrap();
        assert!(publish_directory(&source, &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("recipe")).unwrap(),
            "existing"
        );
        fs::remove_dir_all(&target).unwrap();
        publish_directory(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(target.join("recipe")).unwrap(),
            "candidate"
        );
    }

    #[test]
    fn external_binding_edits_are_detected_without_overwriting_them() {
        let directory = fixture("pkg = 'ed'\n");
        let development = open(directory.path(), false);
        let config = directory.path().join(".config.toml");
        let changed = "pkg = 'other'\n# user note\n";
        fs::write(&config, changed).unwrap();
        assert!(
            development
                .verify_binding()
                .unwrap_err()
                .to_string()
                .contains("changed during the operation")
        );
        assert_eq!(fs::read_to_string(config).unwrap(), changed);
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
            repology: Default::default(),
            root: root.to_path_buf(),
            recipes,
            work: root.join("work"),
            specs: PathBuf::from("SPECS"),
            editor: None,
            author: None,
        };
        let area = workspace.work.join("review");
        fs::create_dir_all(&area).unwrap();
        let contents = "pkg = 'ed'\n# user binding note\n";
        fs::write(area.join(".config.toml"), contents).unwrap();
        let mut development = workspace.existing_development("review").unwrap();
        #[cfg(unix)]
        let identity = fs::metadata(area.join(".lock")).unwrap().ino();
        assert!(!area.join("recipe/SPECS/ed").exists());
        workspace.materialize(&mut development).unwrap();
        assert_eq!(
            development.editor_directory(),
            fs::canonicalize(area.join("recipe/SPECS/ed")).unwrap()
        );
        assert_eq!(
            fs::read_to_string(development.spec().unwrap()).unwrap(),
            "recipe baseline\n"
        );
        // Existing recipe validation must also reuse, not reacquire, this lock.
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
        let development = open(directory.path(), false);
        let lock_path = directory.path().join(".lock");
        fs::rename(&lock_path, directory.path().join("old.lock")).unwrap();
        fs::write(&lock_path, "").unwrap();
        let error = development.verify_binding().unwrap_err();
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
