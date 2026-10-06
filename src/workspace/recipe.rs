// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Read committed main without creating branches or Git working trees.

use super::{baseline, git, invalid};
use fs_err as fs;
use std::{
    io,
    path::{Path, PathBuf},
};

pub(crate) fn select_spec<'a>(
    preferred: Option<&str>,
    files: impl Iterator<Item = &'a Path>,
) -> io::Result<PathBuf> {
    let expected = preferred.map(|name| PathBuf::from(format!("{name}.spec")));
    let specs: Vec<_> = files
        .filter(|path| {
            path.parent() == Some(Path::new(""))
                && path.extension().is_some_and(|ext| ext == "spec")
        })
        .collect();
    if let Some(expected) = expected
        && specs.contains(&expected.as_path())
    {
        return Ok(expected);
    }
    match specs.as_slice() {
        [only] => Ok(only.to_path_buf()),
        [] => Err(io::Error::new(io::ErrorKind::NotFound, "no SPEC found")),
        _ => Err(invalid("multiple SPEC files; select an explicit SPEC")),
    }
}

pub(crate) fn spec_in(directory: &Path, preferred: Option<&str>) -> io::Result<PathBuf> {
    let names = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| PathBuf::from(entry.file_name())))
        .collect::<io::Result<Vec<_>>>()?;
    let path = directory.join(select_spec(preferred, names.iter().map(PathBuf::as_path))?);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid(format!(
            "{} must be a regular SPEC, not a symlink",
            path.display()
        )));
    }
    Ok(path)
}

pub(super) struct Plan {
    repository: PathBuf,
    package: PathBuf,
    revision: String,
}

pub(super) fn prepare(
    repository: &Path,
    destination: &Path,
    specs: &Path,
    package: &str,
) -> io::Result<Option<Plan>> {
    if fs::symlink_metadata(destination).is_ok() {
        if !fs::symlink_metadata(destination)?.is_dir() {
            return Err(invalid("recipe must be an ordinary directory"));
        }
        return Ok(None);
    }
    let revision = git::line(
        repository,
        &["rev-parse", "--verify", "refs/heads/main^{commit}"],
    )?;
    let plan = Plan {
        repository: repository.to_owned(),
        package: specs.join(package),
        revision,
    };
    plan.spec_name()?;
    Ok(Some(plan))
}

impl Plan {
    pub(super) fn spec_name(&self) -> io::Result<PathBuf> {
        let package = self
            .package
            .file_name()
            .expect("validated package")
            .to_string_lossy();
        let files = objects(&self.repository, &self.revision, &self.package)?;
        select_spec(Some(&package), files.iter().map(|file| file.path.as_path()))
    }

    pub(super) fn source(&self) -> io::Result<(PathBuf, String, String)> {
        let path = self.package.join(self.spec_name()?);
        let bytes = git::checked(
            &self.repository,
            ["show", &format!("{}:{}", self.revision, git::text(&path)?)],
        )?;
        Ok((
            self.repository.join(path),
            String::from_utf8(bytes).map_err(io::Error::other)?,
            self.revision.clone(),
        ))
    }

    pub(super) fn create(&self, work: &Path) -> io::Result<()> {
        // Prepare the recipe before publishing it. Baseline publication precedes
        // the rename; a binding-only WORK can retry a failed preparation.
        let temp = tempfile::tempdir_in(work)?;
        let recipe = temp.path().join("recipe");
        let target = recipe
            .join("SPECS")
            .join(self.package.file_name().expect("package"));
        fs::create_dir_all(&target)?;
        for file in objects(&self.repository, &self.revision, &self.package)? {
            let path = target.join(&file.path);
            fs::create_dir_all(path.parent().expect("package file parent"))?;
            fs::write(
                &path,
                git::checked(&self.repository, ["cat-file", "blob", &file.object])?,
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    &path,
                    std::fs::Permissions::from_mode(if file.executable { 0o755 } else { 0o644 }),
                )?;
            }
        }
        let files = baseline::read(&target)?;
        baseline::save(
            &work.join("baseline.toml"),
            &baseline::Baseline {
                files,
                allow_create: false,
            },
        )?;
        super::development::publish_directory(&recipe, &work.join("recipe"))
    }
}

/// Query committed file content rather than trusting a clean-looking index.
pub(super) fn files(
    repository: &Path,
    revision: &str,
    package: &Path,
) -> io::Result<baseline::Files> {
    use sha2::{Digest, Sha256};
    objects(repository, revision, package)?
        .into_iter()
        .map(|file| {
            let bytes = git::checked(repository, ["cat-file", "blob", &file.object])?;
            Ok((
                git::text(&file.path)?.to_owned(),
                baseline::File {
                    sha256: format!("{:x}", Sha256::digest(bytes)),
                    executable: file.executable,
                },
            ))
        })
        .collect()
}

struct Object {
    path: PathBuf,
    object: String,
    executable: bool,
}

fn objects(repository: &Path, revision: &str, package: &Path) -> io::Result<Vec<Object>> {
    let listing = git::checked(
        repository,
        ["ls-tree", "-r", "-z", revision, "--", git::text(package)?],
    )?;
    listing
        .split(|b| *b == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let text = std::str::from_utf8(entry).map_err(io::Error::other)?;
            let (info, name) = text
                .split_once('\t')
                .ok_or_else(|| invalid("invalid Git tree entry"))?;
            let mut info = info.split_whitespace();
            let mode = info.next().unwrap_or("");
            if !matches!(mode, "100644" | "100755") {
                return Err(invalid(format!(
                    "{name}: package contains a link or submodule"
                )));
            }
            let path = Path::new(name)
                .strip_prefix(package)
                .map_err(io::Error::other)?;
            baseline::relative(git::text(path)?)?;
            let object = info
                .nth(1)
                .ok_or_else(|| invalid("missing Git object identity"))?
                .to_owned();
            Ok(Object {
                path: path.to_owned(),
                object,
                executable: mode == "100755",
            })
        })
        .collect()
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn conventional_spec_wins_otherwise_only_one_top_level_spec_is_allowed() {
        let choose = |names: &[&str]| select_spec(Some("pkg"), names.iter().map(Path::new));
        assert_eq!(
            choose(&["other.spec", "pkg.spec"]).unwrap(),
            Path::new("pkg.spec")
        );
        assert_eq!(
            choose(&["other.spec", "nested/sample.spec", "fix.patch"]).unwrap(),
            Path::new("other.spec")
        );
        assert!(choose(&["one.spec", "two.spec"]).is_err());
        assert!(choose(&["nested/sample.spec"]).is_err());
    }
}
