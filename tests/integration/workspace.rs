// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Offline initialization, optional Git clone, and preservation of user-owned configuration.

use super::support::{command, git, output_text, run, success};
use std::{
    ffi::OsStr,
    fs,
    path::Path,
    process::{Command, Output},
};

fn fixture_command(directory: &Path) -> Command {
    super::support::isolated_command(env!("CARGO_BIN_EXE_ruyipack"), directory)
}

fn clone(directory: &Path, path: &Path, url: &OsStr) -> Output {
    fixture_command(directory)
        .arg("init")
        .arg(path)
        .arg("--clone")
        .arg(url)
        .output()
        .unwrap()
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "--quiet"]);
    fs::write(directory.path().join("content.txt"), "recipe fixture\n").unwrap();
    git(directory.path(), &["add", "."]);
    git(
        directory.path(),
        &["commit", "--quiet", "-m", "Fixture baseline"],
    );
    directory
}

#[test]
fn init_clone_materializes_a_local_repository_and_never_repeats_implicitly() {
    let source = repository();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("new workspace");
    let output = clone(
        source.path().parent().unwrap(),
        &root,
        source.path().file_name().unwrap(),
    );
    success(&output);
    assert!(output_text(&output.stdout).contains("Initialized RuyiPack workspace"));
    assert!(output_text(&output.stdout).contains("Cloned recipe repository"));
    let recipes = root.join("openruyi");
    assert_eq!(
        git(&recipes, &["rev-parse", "HEAD"]).stdout,
        git(source.path(), &["rev-parse", "HEAD"]).stdout
    );
    assert_eq!(
        fs::read_to_string(recipes.join("content.txt")).unwrap(),
        "recipe fixture\n"
    );
    fs::write(recipes.join("content.txt"), "local edit\n").unwrap();
    let config = fs::read(root.join(".ruyiconfig/config.toml")).unwrap();
    let repeated = command()
        .current_dir(&root)
        .env("PATH", "")
        .arg("init")
        .output()
        .unwrap();
    success(&repeated);
    assert!(output_text(&repeated.stderr).contains("already initialized"));
    let explicit = clone(&root, Path::new("."), source.path().as_os_str());
    assert_eq!(explicit.status.code(), Some(1), "{explicit:?}");
    assert!(
        output_text(&explicit.stderr).contains("already exists"),
        "{explicit:?}"
    );
    assert_eq!(
        fs::read(root.join(".ruyiconfig/config.toml")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(recipes.join("content.txt")).unwrap(),
        "local edit\n"
    );
}

#[test]
fn failed_clone_retains_initialization_and_explicit_retry_uses_configured_recipes() {
    let source = repository();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("retry workspace");
    let missing = directory.path().join("missing repository");
    let output = clone(directory.path(), &root, missing.as_os_str());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output_text(&output.stdout).contains("Initialized RuyiPack workspace"));
    let diagnostic = output_text(&output.stderr);
    assert!(
        diagnostic.contains("initialization succeeded"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("recipe clone failed"), "{diagnostic}");
    assert!(diagnostic.contains("retained"), "{diagnostic}");
    assert!(root.join(".ruyiconfig/build/Dockerfile").is_file());
    assert!(!root.join("openruyi").exists());
    let config = "recipes = 'nested/custom-recipes'\nwork = 'jobs'\nspecs = 'SPECS'\n";
    let config_path = root.join(".ruyiconfig/config.toml");
    fs::write(&config_path, config).unwrap();
    let repeated = command()
        .current_dir(&root)
        .env("PATH", "")
        .arg("init")
        .output()
        .unwrap();
    success(&repeated);
    assert!(!root.join("nested/custom-recipes").exists());
    let retry = clone(&root, Path::new("."), source.path().as_os_str());
    success(&retry);
    assert!(output_text(&retry.stderr).contains("already initialized"));
    assert_eq!(fs::read_to_string(config_path).unwrap(), config);
    assert!(root.join("nested/custom-recipes/.git").is_dir());
    assert!(!root.join("openruyi").exists());
    assert!(!root.join("jobs").exists());
}

#[test]
fn checkout_failure_retains_real_git_files_and_blocks_clone_retry() {
    let source = repository();
    fs::write(
        source.path().join(".gitattributes"),
        "*.txt filter=fixture\n",
    )
    .unwrap();
    git(source.path(), &["add", ".gitattributes"]);
    git(
        source.path(),
        &["commit", "--quiet", "-m", "Required checkout filter"],
    );
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("checkout failure");
    let global_config = directory.path().join("gitconfig");
    fs::write(
        &global_config,
        "[filter \"fixture\"]\nsmudge = false\nrequired = true\n",
    )
    .unwrap();
    let output = fixture_command(directory.path())
        .env("GIT_CONFIG_GLOBAL", &global_config)
        .arg("init")
        .arg(&root)
        .arg("--clone")
        .arg(source.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output_text(&output.stdout).contains("Initialized RuyiPack workspace"));
    assert!(output_text(&output.stderr).contains("recipe clone failed"));
    let recipes = root.join("openruyi");
    assert!(recipes.join(".git").is_dir());
    assert_eq!(
        fs::read_to_string(recipes.join(".gitattributes")).unwrap(),
        "*.txt filter=fixture\n"
    );
    let git_config = fs::read(recipes.join(".git/config")).unwrap();
    let config = fs::read(root.join(".ruyiconfig/config.toml")).unwrap();
    let retry = clone(&root, Path::new("."), source.path().as_os_str());
    assert_eq!(retry.status.code(), Some(1), "{retry:?}");
    assert!(
        output_text(&retry.stderr).contains("already exists"),
        "{retry:?}"
    );
    assert_eq!(fs::read(recipes.join(".git/config")).unwrap(), git_config);
    assert_eq!(
        fs::read(root.join(".ruyiconfig/config.toml")).unwrap(),
        config
    );
    success(&run(&root, &["init"]));
    assert_eq!(fs::read(recipes.join(".git/config")).unwrap(), git_config);
}

#[test]
fn explicit_clone_refuses_existing_empty_or_nonrepository_destinations() {
    for contents in [None, Some("interrupted clone\n")] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        success(&run(root, &["init"]));
        let recipes = root.join("openruyi");
        fs::create_dir(&recipes).unwrap();
        if let Some(contents) = contents {
            fs::write(recipes.join("keep"), contents).unwrap();
        }
        let output = fixture_command(root)
            .env("PATH", "")
            .args(["init", "--clone", "unavailable-repository"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            output_text(&output.stderr).contains("already exists"),
            "{output:?}"
        );
        assert!(!recipes.join(".git").exists());
        assert_eq!(
            fs::read_dir(&recipes).unwrap().count(),
            usize::from(contents.is_some())
        );
        if let Some(contents) = contents {
            assert_eq!(fs::read_to_string(recipes.join("keep")).unwrap(), contents);
        }
    }
}

#[test]
fn init_materializes_defaults_without_external_tools() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("new workspace");
    let output = command()
        .current_dir(directory.path())
        .env("PATH", "")
        .args(["init", "new workspace"])
        .output()
        .unwrap();
    success(&output);
    assert!(output.stderr.is_empty(), "{output:?}");
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(root.join(".ruyiconfig/config.toml")).unwrap()).unwrap();
    assert_eq!(config["recipes"].as_str(), Some("openruyi"));
    assert_eq!(config["work"].as_str(), Some("work"));
    assert_eq!(config["specs"].as_str(), Some("SPECS"));
    for name in ["Dockerfile", "compose.yaml", "openruyi.cfg", "target.json"] {
        assert_eq!(
            fs::read(root.join(".ruyiconfig/build").join(name)).unwrap(),
            fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("environments/openruyi")
                    .join(name)
            )
            .unwrap(),
            "{name}"
        );
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    let config_path = root.join(".ruyiconfig/config.toml");
    let custom = "recipes = '../unprepared'\nwork = './jobs'\nspecs = 'recipes'\n";
    fs::write(&config_path, custom).unwrap();
    fs::remove_file(root.join(".ruyiconfig/build/Dockerfile")).unwrap();
    let repeated = run(&root, &["init"]);
    success(&repeated);
    assert!(output_text(&repeated.stderr).contains("already initialized"));
    assert!(!output_text(&repeated.stderr).contains("invalid"));
    assert_eq!(fs::read_to_string(config_path).unwrap(), custom);
    assert!(!root.join(".ruyiconfig/build/Dockerfile").exists());
}

#[test]
fn init_never_overwrites_nonempty_or_interrupted_directories() {
    for name in ["notes", ".hidden", ".git"] {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(name), "keep").unwrap();
        let output = run(directory.path(), &["init"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output_text(&output.stderr).contains("empty"), "{output:?}");
        assert_eq!(
            fs::read_to_string(directory.path().join(name)).unwrap(),
            "keep"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join(".ruyiconfig");
    fs::create_dir(&marker).unwrap();
    let output = run(directory.path(), &["init"]);
    success(&output);
    assert!(output_text(&output.stderr).contains("already initialized"));
    assert_eq!(fs::read_dir(&marker).unwrap().count(), 0);
    let output = run(
        directory.path(),
        &["init", "--clone", "unavailable-repository"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output_text(&output.stderr).contains("clone was not attempted"));
    assert!(!directory.path().join("openruyi").exists());
    fs::write(marker.join("config.toml"), "not [toml").unwrap();
    success(&run(directory.path(), &["init"]));
    let output = run(
        directory.path(),
        &["init", "--clone", "unavailable-repository"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output_text(&output.stderr).contains("clone was not attempted"));
    assert!(!directory.path().join("openruyi").exists());
    assert_eq!(
        fs::read_to_string(marker.join("config.toml")).unwrap(),
        "not [toml"
    );
    assert_eq!(fs::read_dir(marker).unwrap().count(), 1);
}

#[test]
fn invalid_managed_paths_are_diagnosed_without_repair() {
    let directory = tempfile::tempdir().unwrap();
    success(&run(directory.path(), &["init"]));
    for (recipes, work, specs) in [
        ("openruyi", "../outside", "SPECS"),
        ("openruyi", "/absolute", "SPECS"),
        ("openruyi", ".ruyiconfig/nested", "SPECS"),
        ("work/recipes", "work", "SPECS"),
        ("openruyi", "openruyi/work", "SPECS"),
        ("openruyi", "work", "../SPECS"),
    ] {
        let contents = format!("recipes = {recipes:?}\nwork = {work:?}\nspecs = {specs:?}\n");
        let path = directory.path().join(".ruyiconfig/config.toml");
        fs::write(&path, &contents).unwrap();
        let output = run(directory.path(), &["init"]);
        success(&output);
        assert!(
            output_text(&output.stderr).contains("invalid"),
            "{contents}: {output:?}"
        );
        assert_eq!(fs::read_to_string(path).unwrap(), contents);
    }
}

#[cfg(unix)]
#[test]
fn init_does_not_follow_marker_or_managed_path_symlinks() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let marker = directory.path().join(".ruyiconfig");
    symlink(external.path(), &marker).unwrap();
    let output = run(directory.path(), &["init"]);
    success(&output);
    assert!(output_text(&output.stderr).contains("already initialized"));
    assert!(fs::symlink_metadata(&marker).unwrap().is_symlink());
    assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    let output = run(
        directory.path(),
        &["init", "--clone", "unavailable-repository"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output_text(&output.stderr).contains("clone was not attempted"));
    assert!(fs::symlink_metadata(&marker).unwrap().is_symlink());
    assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    assert!(!directory.path().join("openruyi").exists());
    fs::remove_file(marker).unwrap();
    success(&run(directory.path(), &["init"]));
    symlink(external.path(), directory.path().join("work")).unwrap();
    let output = run(directory.path(), &["init"]);
    success(&output);
    assert!(
        output_text(&output.stderr).contains("invalid"),
        "{output:?}"
    );
    assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
}
