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

#[track_caller]
fn assert_clone_not_attempted(root: &Path) {
    let output = run(root, &["init", "--clone", "unavailable-repository"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output_text(&output.stderr).contains("clone was not attempted"));
    assert!(!root.join("openruyi").exists());
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
    let pr_template = root.join(".ruyiconfig/pr.md");
    assert!(
        fs::read_to_string(&pr_template)
            .unwrap()
            .contains("{{summary}}")
    );
    fs::write(&pr_template, "User template").unwrap();
    let config = fs::read(root.join(".ruyiconfig/config.toml")).unwrap();
    let repeated = command()
        .current_dir(&root)
        .env("PATH", "")
        .arg("init")
        .output()
        .unwrap();
    success(&repeated);
    assert!(output_text(&repeated.stderr).contains("already initialized"));
    assert_eq!(fs::read_to_string(&pr_template).unwrap(), "User template");
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
    assert!(
        output_text(&output.stderr).contains("author is missing or invalid"),
        "{output:?}"
    );
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
    assert_clone_not_attempted(directory.path());
    fs::write(marker.join("config.toml"), "not [toml").unwrap();
    success(&run(directory.path(), &["init"]));
    assert_clone_not_attempted(directory.path());
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
    assert_clone_not_attempted(directory.path());
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

#[cfg(unix)]
#[test]
fn recipe_editor_updates_real_files_without_staging_and_propagates_failure() {
    let root = tempfile::tempdir().unwrap();
    let area = super::support::recipe_workspace(
        root.path(),
        "review",
        "ed",
        include_str!("../fixtures/ed.spec"),
    );
    let editor = root.path().join("editor.sh");
    fs::write(
        &editor,
        "printf 'patch edit\\n' > \"$1/fix.patch\"\nexit 7\n",
    )
    .unwrap();
    let output = run(
        root.path(),
        &[
            "open",
            "review",
            "--editor",
            &format!("/bin/sh {}", shell_words::quote(&editor.to_string_lossy())),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output_text(&output.stderr).contains("editor exited"));
    assert_eq!(
        fs::read_to_string(area.join("recipe/SPECS/ed/fix.patch")).unwrap(),
        "patch edit\n"
    );
    assert!(!area.join("stage").exists());
    fs::write(&editor, "test -f \"$1/fix.patch\"\n").unwrap();
    success(&run(
        root.path(),
        &[
            "open",
            "review",
            "--editor",
            &format!("/bin/sh {}", shell_words::quote(&editor.to_string_lossy())),
        ],
    ));
    for conflicting in ["--apply", "--hash", "--diff", "--check"] {
        assert_eq!(
            run(root.path(), &["open", "review", conflicting])
                .status
                .code(),
            Some(2)
        );
    }
}

#[test]
fn history_cleanup_reports_partial_failure_and_preserves_current_and_authoring() {
    let root = tempfile::tempdir().unwrap();
    let area = super::support::authoring_workspace(root.path(), "ed", "ed", "author input\n");
    let current = area.join("build");
    fs::create_dir(&current).unwrap();
    fs::write(current.join("keep"), "current").unwrap();
    let history = area.join("build-history");
    for name in ["good", "broken"] {
        fs::create_dir_all(history.join(name)).unwrap();
    }
    fs::write(
        history.join("good/receipt.json"),
        r#"{"resources_retained":false}"#,
    )
    .unwrap();
    fs::write(history.join("broken/receipt.json"), "broken receipt").unwrap();
    let output = run(
        root.path(),
        &["clean", "ed", "--history", "--force", "--format=toml"],
    );
    assert_eq!(output.status.code(), Some(1));
    let report = super::support::machine_report(&output);
    assert_eq!(report["results"].as_array().unwrap().len(), 2);
    assert!(!history.join("good").exists());
    assert!(history.join("broken/receipt.json").exists());
    assert_eq!(fs::read_to_string(current.join("keep")).unwrap(), "current");
    assert_eq!(
        fs::read_to_string(area.join("ed.toml")).unwrap(),
        "author input\n"
    );
    fs::write(
        history.join("broken/receipt.json"),
        r#"{"resources_retained":false}"#,
    )
    .unwrap();
    for _ in 0..2 {
        success(&run(
            root.path(),
            &["clean", "ed", "--history", "--force", "--format=toml"],
        ));
    }
}

#[cfg(unix)]
#[test]
fn configured_editor_precedence_uses_git_and_shell_arguments() {
    let root = tempfile::tempdir().unwrap();
    let area = super::support::recipe_workspace(
        root.path(),
        "ed",
        "ed",
        include_str!("../fixtures/ed.spec"),
    );
    let editor = root.path().join("editor with spaces.sh");
    fs::write(&editor, "printf '%s' \"$1\" > \"$2/editor-used\"\n").unwrap();
    let config = root.path().join(".ruyiconfig/config.toml");
    let original = fs::read_to_string(&config).unwrap();
    git(
        &root.path().join("openruyi"),
        &["config", "core.editor", "/bin/sh \"$RPK_TEST_EDITOR\" git"],
    );
    for (setting, expected) in [
        ("git", "git"),
        ("env", "env"),
        ("workspace", "workspace"),
        ("explicit", "explicit"),
    ] {
        let mut command =
            super::support::isolated_command(env!("CARGO_BIN_EXE_ruyipack"), root.path());
        command.args(["open", "ed"]).env("RPK_TEST_EDITOR", &editor);
        if setting != "git" {
            command.env("GIT_EDITOR", "/bin/sh \"$RPK_TEST_EDITOR\" env");
        }
        if matches!(setting, "workspace" | "explicit") {
            fs::write(
                &config,
                format!("{original}\neditor = '/bin/sh \"$RPK_TEST_EDITOR\" workspace'\n"),
            )
            .unwrap();
        }
        if setting == "explicit" {
            command.args(["--editor", "/bin/sh \"$RPK_TEST_EDITOR\" explicit"]);
        }
        success(&command.output().unwrap());
        assert_eq!(
            fs::read_to_string(area.join("recipe/SPECS/ed/editor-used")).unwrap(),
            expected
        );
    }
}

#[test]
fn inspect_work_state_is_independent_of_report_format() {
    use super::support::{machine_report, recipe_workspace};
    let root = tempfile::tempdir().unwrap();
    recipe_workspace(
        root.path(),
        "seed",
        "ed",
        include_str!("../fixtures/ed.spec"),
    );
    success(&run(root.path(), &["new", "fresh"]));
    for (work, present) in [("fresh", false), ("ed", true)] {
        let human = run(root.path(), &["inspect", work]);
        let machine = run(root.path(), &["inspect", work, "--format=toml"]);
        success(&human);
        success(&machine);
        assert_eq!(human.status.code(), machine.status.code());
        assert_eq!(
            machine_report(&machine)["work"]["spec_present"].as_bool(),
            Some(present)
        );
        assert!(
            String::from_utf8_lossy(&human.stdout).contains(&format!("SPEC present: {present}"))
        );
    }
    assert!(!root.path().join("work/ed/recipe").exists());
    for args in [
        vec!["inspect", "fresh", "--editable", "--all"],
        vec!["inspect", "--spec", "missing.spec"],
        vec!["inspect", "--spec", "missing.spec", "--format=toml"],
    ] {
        assert!(!run(root.path(), &args).status.success());
    }
}
