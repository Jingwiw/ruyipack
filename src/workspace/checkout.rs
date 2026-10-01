// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Git owns checkout identity and branches; no second copy is stored in workspace state.

use super::invalid;
use fs_err as fs;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

pub(super) struct Plan {
    recipes: PathBuf,
    checkout: PathBuf,
    package: PathBuf,
    commit: String,
    branch: String,
    sparse: Vec<PathBuf>,
}

pub(super) fn prepare(
    recipes: &Path,
    checkout: &Path,
    specs: &Path,
    package: &str,
    work: &str,
) -> io::Result<Option<Plan>> {
    match fs::symlink_metadata(checkout) {
        Ok(_) => {
            validate(recipes, checkout, specs, package)?;
            return Ok(None);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    repository_root(recipes)?;
    let commit = line(
        recipes,
        &["rev-parse", "--verify", "refs/heads/main^{commit}"],
    )?;
    let package = specs.join(package);
    committed_package(recipes, &commit, &package)?;
    let identity = format!(
        "{:x}",
        Sha256::digest(checkout.as_os_str().as_encoded_bytes())
    );
    let branch = format!("ruyipack/{}/{work}", &identity[..16]);
    checked(recipes, ["check-ref-format", "--branch", &branch])?;
    let mut sparse = Vec::new();
    let mut parent = PathBuf::new();
    // Cone mode includes ancestor files, but not their sibling directories. Preserve
    // those directories too, even when the configured SPEC root is nested.
    for component in specs.components() {
        let name = component.as_os_str();
        if name == "." {
            continue;
        }
        let tree = if parent.as_os_str().is_empty() {
            commit.clone()
        } else {
            format!("{commit}:{}", text(&parent)?)
        };
        for (mode, child) in tree_entries(recipes, [&*tree])? {
            if mode == "040000" && OsStr::new(&child) != name {
                sparse.push(parent.join(child));
            }
        }
        parent.push(name);
        // A new repository may have no SPEC directory yet.
        if tree_entries(recipes, [&*commit, "--", text(&parent)?])?.is_empty() {
            break;
        }
    }
    sparse.push(package.clone());
    Ok(Some(Plan {
        recipes: recipes.to_path_buf(),
        checkout: checkout.to_path_buf(),
        package,
        commit,
        branch,
        sparse,
    }))
}

impl Plan {
    pub(super) fn repository(&self) -> &Path {
        &self.recipes
    }

    pub(super) fn spec_name(&self) -> io::Result<PathBuf> {
        let name = self
            .package
            .file_name()
            .ok_or_else(|| invalid("package has no name"))?;
        let mut spec = name.to_os_string();
        spec.push(".spec");
        let spec = PathBuf::from(spec);
        let path = self.package.join(&spec);
        let entries = tree_entries(&self.recipes, [&*self.commit, "--", text(&path)?])?;
        if !matches!(entries.as_slice(), [(mode, _)] if matches!(mode.as_str(), "100644" | "100755"))
        {
            return Err(invalid(format!(
                "{} is not an ordinary SPEC in committed main; use --pkgname to bind another package or create its recipe explicitly",
                self.recipes.join(&path).display()
            )));
        }
        Ok(spec)
    }

    pub(super) fn source(&self) -> io::Result<(PathBuf, String, String)> {
        let path = self.package.join(self.spec_name()?);
        let object = format!("{}:{}", self.commit, text(&path)?);
        let bytes = checked(&self.recipes, ["show", &object])?;
        let source = String::from_utf8(bytes).map_err(io::Error::other)?;
        Ok((self.recipes.join(path), source, self.commit.clone()))
    }

    pub(super) fn create(&self) -> io::Result<()> {
        let result = (|| {
            if line(
                &self.recipes,
                &["rev-parse", "--verify", "refs/heads/main^{commit}"],
            )? != self.commit
            {
                return Err(invalid(
                    "recipe main changed after checkout planning; retry",
                ));
            }
            clean_package(&self.recipes, &self.package)?;
            checked(
                &self.recipes,
                [
                    OsStr::new("worktree"),
                    OsStr::new("add"),
                    OsStr::new("--no-checkout"),
                    OsStr::new("-b"),
                    OsStr::new(&self.branch),
                    self.checkout.as_os_str(),
                    OsStr::new(&self.commit),
                ],
            )?;
            let mut arguments = vec![
                OsStr::new("sparse-checkout"),
                OsStr::new("set"),
                OsStr::new("--cone"),
                OsStr::new("--skip-checks"),
                OsStr::new("--"),
            ];
            arguments.extend(self.sparse.iter().map(|path| path.as_os_str()));
            checked(&self.checkout, arguments)?;
            // --no-checkout leaves the index empty; sparse-checkout set alone does
            // not materialize it. Populate only this newly created worktree.
            checked(&self.checkout, ["read-tree", "-mu", &self.commit])?;
            fs::create_dir_all(self.checkout.join(&self.package))?;
            Ok(())
        })();
        result.map_err(|error: io::Error| {
            io::Error::new(
                error.kind(),
                format!(
                    "{error}; checkout creation may be incomplete at {}; Git branch: {}; inspect with git -C {} worktree list; retained files were not deleted",
                    self.checkout.display(),
                    self.branch,
                    shell_words::quote(&self.recipes.to_string_lossy())
                ),
            )
        })
    }
}

pub(super) fn validate(
    recipes: &Path,
    checkout: &Path,
    specs: &Path,
    package: &str,
) -> io::Result<()> {
    repository_root(recipes)?;
    if !fs::symlink_metadata(checkout)?.is_dir() {
        return Err(invalid(format!(
            "{} must be a directory, not a symlink",
            checkout.display()
        )));
    }
    if !fs::symlink_metadata(checkout.join(".git"))?.is_file() {
        return Err(invalid(format!(
            "{} must be a linked Git worktree with a regular .git pointer file",
            checkout.display()
        )));
    }
    repository_root(checkout)?;
    let common = |path: &Path| {
        fs::canonicalize(line(
            path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?)
    };
    if common(recipes)? != common(checkout)? {
        return Err(invalid(format!(
            "{} belongs to a different Git repository than {}",
            checkout.display(),
            recipes.display()
        )));
    }
    // A developer may switch branches or detach HEAD; never reset their checkout.
    let commit = line(checkout, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let package = specs.join(package);
    committed_package(checkout, &commit, &package)?;
    ordinary_package(checkout, &package)
}

fn repository_root(path: &Path) -> io::Result<()> {
    let actual = line(
        path,
        &["rev-parse", "--path-format=absolute", "--show-toplevel"],
    )?;
    if fs::canonicalize(path)? != fs::canonicalize(actual)? {
        return Err(invalid(format!(
            "{} must be the Git repository root, not a directory inside another repository",
            path.display()
        )));
    }
    Ok(())
}

fn committed_package(repo: &Path, commit: &str, package: &Path) -> io::Result<()> {
    let mut ancestor = PathBuf::new();
    for part in package.components() {
        ancestor.push(part);
        for (mode, _) in tree_entries(repo, [commit, "--", text(&ancestor)?])? {
            if mode != "040000" {
                return Err(invalid(format!(
                    "committed package ancestor {} must be a directory, not a symlink, file or submodule",
                    ancestor.display()
                )));
            }
        }
    }
    for (mode, path) in tree_entries(repo, ["-r", commit, "--", text(package)?])? {
        if !matches!(mode.as_str(), "100644" | "100755") {
            return Err(invalid(format!(
                "committed package path {path} must be an ordinary file, not a symlink or submodule"
            )));
        }
    }
    Ok(())
}

fn clean_package(repo: &Path, package: &Path) -> io::Result<()> {
    ordinary_package(repo, package)?;
    // status intentionally trusts these index flags. Do not let them hide source
    // edits; absent skip-worktree entries remain valid in a sparse recipe checkout.
    let entries = checked(repo, ["ls-files", "-v", "-z", "--", text(package)?])?;
    for entry in entries
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let path =
            std::str::from_utf8(entry.get(2..).unwrap_or_default()).map_err(io::Error::other)?;
        if entry[0].is_ascii_lowercase() || entry[0] == b'S' && repo.join(path).try_exists()? {
            return Err(invalid(format!(
                "{} uses assume-unchanged or a materialized skip-worktree entry; restore normal Git index tracking before creating a checkout so package edits cannot be missed",
                repo.join(path).display()
            )));
        }
    }
    let status = checked(
        repo,
        [
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignored=matching",
            "--ignore-submodules=none",
            "--",
            text(package)?,
        ],
    )?;
    if !status.is_empty() {
        return Err(invalid(format!(
            "{} has uncommitted or ignored package content; commit the intended recipe first (nothing was copied):\n{}",
            repo.join(package).display(),
            String::from_utf8_lossy(&status).trim_end()
        )));
    }
    Ok(())
}

fn ordinary_package(root: &Path, package: &Path) -> io::Result<()> {
    let mut path = root.to_path_buf();
    for part in package.components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(invalid(format!(
                    "{} must be a directory, not a symlink",
                    path.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    let mut pending = vec![path];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if !kind.is_file() {
                return Err(invalid(format!(
                    "{} must be an ordinary package file, not a symlink or special file",
                    entry.path().display()
                )));
            }
        }
    }
    Ok(())
}

fn tree_entries<'a>(
    repo: &Path,
    args: impl IntoIterator<Item = &'a str>,
) -> io::Result<Vec<(String, String)>> {
    let output = checked(repo, ["ls-tree", "-z"].into_iter().chain(args))?;
    output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = std::str::from_utf8(entry).map_err(io::Error::other)?;
            let (metadata, name) = entry
                .split_once('\t')
                .ok_or_else(|| invalid("Git ls-tree returned an invalid entry"))?;
            let mode = metadata.split(' ').next().unwrap_or_default();
            Ok((mode.to_owned(), name.to_owned()))
        })
        .collect()
}

fn line(repo: &Path, args: &[&str]) -> io::Result<String> {
    let output = String::from_utf8(checked(repo, args)?).map_err(io::Error::other)?;
    Ok(output.strip_suffix('\n').unwrap_or(&output).to_owned())
}

fn text(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| {
        invalid(format!(
            "Git selection path {} must be UTF-8",
            path.display()
        ))
    })
}

fn checked(repo: &Path, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> io::Result<Vec<u8>> {
    let args: Vec<_> = args.into_iter().collect();
    let output = git(repo, &args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(command_error(
            repo,
            &args[0].as_ref().to_string_lossy(),
            &output,
        ))
    }
}

fn git(repo: &Path, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> io::Result<Output> {
    git_with_budget(repo, args, Duration::from_secs(30))
}

pub(super) fn clone_repository(destination: &Path, url: &OsStr) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(invalid(format!(
                "{} already exists; inspect it or choose an unused recipes path before retrying",
                destination.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // Relative local URLs use the caller's directory, not the destination's.
    // Git creates missing destination parents itself.
    let current = std::env::current_dir()?;
    let output = git_with_budget(
        &current,
        [
            OsStr::new("clone"),
            OsStr::new("--"),
            url,
            destination.as_os_str(),
        ],
        Duration::from_secs(300),
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(&current, "clone", &output))
    }
}

fn git_with_budget(
    repo: &Path,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    budget: Duration,
) -> io::Result<Output> {
    let mut command = Command::new("git");
    command
        .args([
            "--literal-pathspecs",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-C",
        ])
        .arg(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1");
    for key in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(key);
    }
    crate::host_process::capture(&mut command, budget, 8 * 1024 * 1024).map_err(|error| {
        io::Error::new(error.kind(), format!("Git in {}: {error}", repo.display()))
    })
}

fn command_error(repo: &Path, command: &str, output: &Output) -> io::Error {
    io::Error::other(format!(
        "git {command} in {} failed ({}): {}",
        repo.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim_end()
    ))
}
