// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Git-backed development areas, scaffold identity, and the existing gen consumer.

use super::support::{assert_file, success};

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use super::support;
use super::support::git;
use support::output_text;

fn command(directory: &Path) -> Command {
    let mut command = support::isolated_command(env!("CARGO_BIN_EXE_ruyipack"), directory);
    command
        .env("TZ", "UTC")
        .env("GIT_AUTHOR_NAME", "Packager \"A\" \\测试")
        .env("GIT_AUTHOR_EMAIL", "packager@example.org");
    command
}

fn run(directory: &Path, args: &[&str]) -> Output {
    command(directory).args(args).output().unwrap()
}

fn workspace() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success(&run(root, &["init"]));
    let config_path = root.join(".ruyiconfig/config.toml");
    let mut config: toml::Table =
        toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    config.insert(
        "author".into(),
        toml::Value::String("Packager \"A\" \\测试 <packager@example.org>".into()),
    );
    fs::write(config_path, toml::to_string(&config).unwrap()).unwrap();
    let repo = root.join("openruyi");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    for (path, contents) in [
        ("SPECS/ed/ed.spec", include_str!("../fixtures/ed.spec")),
        ("SPECS/other/ed.spec", "other package\n"),
        ("SPECS/README", "shared package guidance\n"),
        ("scripts/nested/helper", "helper\n"),
        ("policies/rules", "rules\n"),
        ("README", "repository guidance\n"),
    ] {
        let path = repo.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "Fixture baseline"]);
    directory
}

fn document(output: &Output) -> toml::Value {
    success(output);
    toml::from_str(output_text(&output.stdout)).unwrap()
}

fn edit_version(directory: &Path, target: &str) -> String {
    document(&run(
        directory,
        &[
            "inspect",
            target,
            "--field",
            "package.version",
            "--editable",
        ],
    ))["package"]["version"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn version_spec(version: &str) -> String {
    let original = include_str!("../fixtures/ed.spec");
    let field = "Version:        1.22.5";
    assert_eq!(original.matches(field).count(), 1);
    original.replace(field, &format!("Version:        {version}"))
}

#[test]
fn edit_work_reads_its_saved_package_recipe_and_only_publishes_there() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    for work in ["ed-test", "ed-other"] {
        success(&run(root, &["new", work, "--pkgname", "ed"]));
        seed_recipe(root, work);
    }
    fs::write(
        root.join("work/ed-test/ed.toml"),
        include_str!("../../examples/ed/ed.toml"),
    )
    .unwrap();
    let area = root.join("work/ed-test");
    seed_recipe(root, "ed-test");
    let target = area.join("recipe/SPECS/ed/ed.spec");
    let original = version_spec("1.22.6");
    fs::write(&target, &original).unwrap();
    let retained: Vec<_> = [
        repo.join("SPECS/ed/ed.spec"),
        root.join("work/ed-other/recipe/SPECS/ed/ed.spec"),
        root.join("work/ed-other/ed.toml"),
        root.join("work/ed-other/.config.toml"),
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(&path).unwrap();
        (path, bytes)
    })
    .collect();
    let nested = repo.join("scripts/nested");
    for cwd in [root, nested.as_path()] {
        assert_eq!(edit_version(cwd, "ed-test"), "1.22.6");
    }

    let preview = run(
        &nested,
        &[
            "edit",
            "ed-test",
            "--set",
            "package.version=1.22.7",
            "--diff",
        ],
    );
    success(&preview);
    let diff = output_text(&preview.stdout);
    assert!(diff.contains("-Version:        1.22.6"), "{diff}");
    assert!(diff.contains("+Version:        1.22.7"), "{diff}");
    assert_file(&target, &original);

    for (path, bytes) in &retained {
        assert_eq!(fs::read(path).unwrap(), *bytes, "{}", path.display());
    }

    for (cwd, version) in [(root, "1.22.7"), (nested.as_path(), "1.22.8")] {
        success(&run(
            cwd,
            &[
                "edit",
                "ed-test",
                "--set",
                &format!("package.version={version}"),
                "--apply",
            ],
        ));
        assert_file(&target, &version_spec(version));
        for (path, bytes) in &retained {
            assert_eq!(fs::read(path).unwrap(), *bytes, "{}", path.display());
        }
    }
}

#[test]
fn edit_work_wins_over_a_bare_file_and_explicit_paths_still_select_files() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "ed-test", "--pkgname", "ed"]));
    seed_recipe(root, "ed-test");
    let bare = root.join("ed-test");
    let spec = root.join("ed-test.spec");
    fs::write(&bare, version_spec("9.0")).unwrap();
    fs::write(&spec, version_spec("8.0")).unwrap();
    assert_eq!(edit_version(root, "ed-test"), "1.22.5");
    for (path, expected) in [("./ed-test", "9.0"), ("ed-test.spec", "8.0")] {
        assert_eq!(
            document(&run(
                root,
                &[
                    "inspect",
                    "--spec",
                    path,
                    "--editable",
                    "--field",
                    "package.version"
                ]
            ))["package"]["version"]
                .as_str(),
            Some(expected)
        );
    }
    for (path, version, stage) in [
        ("./ed-test", "9.1", "bare-stage"),
        ("ed-test.spec", "8.1", "spec-stage"),
    ] {
        success(&run(
            root,
            &[
                "edit",
                "--spec",
                path,
                "--set",
                &format!("package.version={version}"),
                "--prepare",
                stage,
                "--apply",
            ],
        ));
    }
    assert_file(bare, &version_spec("9.1"));
    assert_file(spec, &version_spec("8.1"));
    assert_file(
        root.join("work/ed-test/recipe/SPECS/ed/ed.spec"),
        include_str!("../fixtures/ed.spec"),
    );
}

#[test]
fn first_edit_creates_a_package_bound_work_without_changing_recipes() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    let original = fs::read(repo.join("SPECS/ed/ed.spec")).unwrap();
    let area = root.join("work/first-ed");
    let target = area.join("recipe/SPECS/ed/ed.spec");
    assert!(!area.exists());
    success(&run(
        root,
        &[
            "edit",
            "first-ed",
            "--pkgname",
            "ed",
            "--set",
            "package.version=1.22.6",
            "--apply",
        ],
    ));
    assert!(area.join("recipe/SPECS/ed/ed.spec").is_file());
    assert_file(&target, &version_spec("1.22.6"));
    assert_eq!(edit_version(root, "first-ed"), "1.22.6");
    success(&run(
        &repo.join("scripts/nested"),
        &[
            "edit",
            "first-ed",
            "--set",
            "package.version=1.22.7",
            "--apply",
        ],
    ));
    assert_file(&target, &version_spec("1.22.7"));
    let rebind = run(
        root,
        &[
            "edit",
            "first-ed",
            "--pkgname",
            "other",
            "--set",
            "package.version=1.22.8",
        ],
    );
    assert_eq!(rebind.status.code(), Some(1), "{rebind:?}");
    assert_file(&target, &version_spec("1.22.7"));
    assert_eq!(fs::read(repo.join("SPECS/ed/ed.spec")).unwrap(), original);
}

#[test]
fn edit_missing_or_misnamed_package_spec_is_rejected_before_copying_package_files() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    fs::create_dir(repo.join("SPECS/no-spec")).unwrap();
    fs::write(repo.join("SPECS/no-spec/README"), "no SPEC yet\n").unwrap();
    fs::create_dir(repo.join("SPECS/many-specs")).unwrap();
    for name in ["one.spec", "two.spec"] {
        fs::write(
            repo.join("SPECS/many-specs").join(name),
            include_str!("../fixtures/ed.spec"),
        )
        .unwrap();
    }
    git(&repo, &["add", "SPECS/no-spec", "SPECS/many-specs"]);
    git(
        &repo,
        &["commit", "--quiet", "-m", "Nonunique SPEC fixtures"],
    );
    let bare = root.join("missing-pkg");
    let original = include_str!("../fixtures/ed.spec");
    fs::write(&bare, original).unwrap();
    for work in ["missing-pkg", "no-spec", "many-specs"] {
        for args in [
            vec!["inspect", work, "--field", "package.version", "--editable"],
            vec!["edit", work, "--set", "package.version=1.22.6", "--diff"],
            vec!["edit", work, "--set", "package.version=1.22.6"],
            vec!["gen", work, "--offline", "--check", "--format", "toml"],
        ] {
            let output = run(root, &args);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            if args[0] == "gen" {
                assert!(output.stderr.is_empty(), "{output:?}");
                let report = support::machine_report(&output);
                assert!(report.get("valid").is_none());
                assert_eq!(report["success"].as_bool(), Some(false));
                assert_eq!(report["error"]["code"].as_str(), Some("input-read"));
            }
            assert!(!root.join("work").exists());
            assert_file(&bare, original);
            assert_file(repo.join("SPECS/ed/ed.spec"), original);
        }
    }
    let dotted = repo.join("SPECS/ed.plus");
    fs::create_dir(&dotted).unwrap();
    fs::write(
        dotted.join("ed.plus.spec"),
        original.replace("Name:           ed", "Name:           ed.plus"),
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "Dotted package name"]);
    assert_eq!(edit_version(root, "ed.plus"), "1.22.5");
    let area = root.join("work/ed.plus");
    let binding: toml::Table =
        toml::from_str(&fs::read_to_string(area.join(".config.toml")).unwrap()).unwrap();
    assert_eq!(binding["pkg"].as_str(), Some("ed.plus"));
    assert!(!area.join("recipe").exists());
    success(&run(
        root,
        &[
            "edit",
            "ed.plus",
            "--set",
            "package.version=1.22.6",
            "--apply",
        ],
    ));
    assert!(area.join("recipe/SPECS/ed.plus/ed.plus.spec").is_file());
    assert_eq!(edit_version(root, "ed.plus"), "1.22.6");
}

#[test]
fn edit_work_refuses_a_locked_binding_even_for_read_only_operations() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "ed-test", "--pkgname", "ed"]));
    let area = root.join("work/ed-test");
    seed_recipe(root, "ed-test");
    let target = area.join("recipe/SPECS/ed/ed.spec");
    let binding = area.join(".config.toml");
    let saved = fs::read(&binding).unwrap();
    let manifest = fs::read(area.join("ed.toml")).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(area.join(".lock"))
        .unwrap();
    lock.try_lock().unwrap();
    for args in [
        vec![
            "inspect",
            "ed-test",
            "--field",
            "package.version",
            "--editable",
        ],
        vec![
            "edit",
            "ed-test",
            "--set",
            "package.version=1.22.6",
            "--diff",
        ],
        vec!["edit", "ed-test", "--set", "package.version=1.22.6"],
    ] {
        let output = run(root, &args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output_text(&output.stderr).contains("lock"), "{output:?}");
        assert_file(&target, include_str!("../fixtures/ed.spec"));
        assert_eq!(fs::read(&binding).unwrap(), saved);
        assert_eq!(fs::read(area.join("ed.toml")).unwrap(), manifest);
    }
    lock.unlock().unwrap();
    assert_eq!(edit_version(root, "ed-test"), "1.22.5");
}

#[test]
fn comment_modes_share_fields_and_filled_scaffolds_use_gen() {
    let directory = workspace();
    let root = directory.path();
    let standard = run(root, &["new", "ed", "--stdout"]);
    let full = run(root, &["new", "ed", "--stdout", "--comments", "full"]);
    let mut scaffold = document(&standard);
    assert_eq!(scaffold, document(&full));
    assert!(full.stdout.len() > standard.stdout.len());
    assert_eq!(scaffold["package"]["name"].as_str(), Some("ed"));
    assert_eq!(
        scaffold["spec"]["contributors"][0].as_str(),
        Some("Packager \"A\" \\测试 <packager@example.org>")
    );
    assert_eq!(
        scaffold["spec"]["copyright-years"].as_str(),
        Some(time::OffsetDateTime::now_utc().year().to_string().as_str())
    );
    assert_eq!(scaffold["package"]["version"].as_str(), Some(""));
    assert!(scaffold["sources"]["0"].get("sha256").is_none());
    assert!(scaffold["package"]["vcs"].as_table().unwrap().is_empty());
    assert!(scaffold.get("build").is_none());
    assert!(scaffold.get("subpackages").is_none());
    assert!(!root.join("work").exists());
    support::authoring_workspace(root, "ed", "ed", output_text(&standard.stdout));
    let incomplete = run(root, &["gen", "ed", "--offline"]);
    assert_eq!(incomplete.status.code(), Some(1));
    assert!(incomplete.stdout.is_empty());
    assert!(!root.join("ed.spec").exists());

    let filled: toml::Value = toml::from_str(include_str!("../../examples/ed/ed.toml")).unwrap();
    // Populate the scaffold's existing authoring fields, then select the already supported build system.
    for section in ["spec", "package", "sources", "build-requires"] {
        for key in scaffold[section].as_table().unwrap().keys() {
            assert!(filled[section].get(key).is_some(), "{section}.{key}");
        }
        scaffold[section] = filled[section].clone();
    }
    scaffold
        .as_table_mut()
        .unwrap()
        .insert("build".into(), filled["build"].clone());
    fs::write(
        root.join("work/ed/ed.toml"),
        toml::to_string(&scaffold).unwrap(),
    )
    .unwrap();
    let generated = command(root)
        .env("GIT_AUTHOR_NAME", "Different author")
        .args(["gen", "ed", "--stdout"])
        .output()
        .unwrap();
    success(&generated);
    assert_eq!(
        output_text(&generated.stdout),
        include_str!("../fixtures/ed.spec")
    );
}

#[test]
fn new_preserves_manual_work_and_only_explicitly_overwrites_the_scaffold() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "demo", "--pkgname", "demo"]));
    let area = root.join("work/demo");
    let target = area.join("demo.toml");
    let expected = fs::read(&target).unwrap();
    let recipe = area.join("recipe/SPECS/demo");
    let config = fs::read(area.join(".config.toml")).unwrap();
    fs::write(recipe.join("README"), "local recipe edit\n").unwrap();
    fs::write(&target, "# manual content\n").unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(area.join(".lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let busy = run(root, &["new", "demo", "--pkgname", "demo", "--force"]);
    assert_eq!(busy.status.code(), Some(1), "{busy:?}");
    assert_file(&target, "# manual content\n");
    lock.unlock().unwrap();
    let repo = root.join("openruyi");
    fs::write(repo.join("README"), "new upstream commit\n").unwrap();
    git(
        &repo,
        &["commit", "--quiet", "-am", "Advance recipe baseline"],
    );
    let conflict = run(root, &["new", "demo", "--pkgname", "demo"]);
    assert_eq!(conflict.status.code(), Some(1));
    assert!(output_text(&conflict.stderr).contains("already exists with different content"));
    for action in ["--stdout", "--diff", "--skip-existing"] {
        success(&run(root, &["new", "demo", "--pkgname", "demo", action]));
        assert_file(&target, "# manual content\n");
    }
    let conflict = run(root, &["new", "demo", "--pkgname", "ed", "--force"]);
    assert_eq!(conflict.status.code(), Some(1), "{conflict:?}");
    assert_eq!(fs::read(area.join(".config.toml")).unwrap(), config);
    assert_file(&target, "# manual content\n");
    success(&run(root, &["new", "demo", "--pkgname", "demo", "--force"]));
    assert_eq!(fs::read(&target).unwrap(), expected);
    assert_file(recipe.join("README"), "local recipe edit\n");
    assert_eq!(
        run(
            root,
            &["new", "demo", "--pkgname", "demo", "--stdout", "--force"]
        )
        .status
        .code(),
        Some(2)
    );
}

#[test]
fn edit_and_gen_share_the_authoring_input() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "review", "--pkgname", "ed"]));
    seed_recipe(root, "review");
    let area = root.join("work/review");
    let manifest = area.join("ed.toml");
    fs::write(&manifest, include_str!("../../examples/ed/ed.toml")).unwrap();
    success(&run(
        root,
        &["edit", "review", "--set=package.version=1.22.7"],
    ));
    let document: toml::Table = toml::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    assert_eq!(document["package"]["version"].as_str(), Some("1.22.7"));
    let generated = run(root, &["gen", "review", "--offline", "--stdout"]);
    success(&generated);
    assert!(String::from_utf8_lossy(&generated.stdout).contains("Version:        1.22.7"));
    assert_file(
        area.join("recipe/SPECS/ed/ed.spec"),
        include_str!("../fixtures/ed.spec"),
    );
    let contents = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        format!("{contents}\n[build.stages.build]\nappend = 'echo from-authoring'\n"),
    )
    .unwrap();
    let edited = run(root, &["edit", "review", "--stdout"]);
    success(&edited);
    assert!(String::from_utf8_lossy(&edited.stdout).contains("echo from-authoring"));
    let before = fs::read(&manifest).unwrap();
    let unsupported = run(
        directory.path(),
        &[
            "edit",
            "review",
            "--set",
            "spec.copyright-holders=['Other holder']",
        ],
    );
    assert_eq!(unsupported.status.code(), Some(1));
    assert_eq!(fs::read(&manifest).unwrap(), before);
}

#[test]
fn edit_copy_output_cannot_replace_work_inputs_or_binding_state() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "review", "--pkgname", "ed"]));
    let area = root.join("work/review");
    seed_recipe(root, "review");
    let manifest = area.join("ed.toml");
    fs::write(&manifest, include_str!("../../examples/ed/ed.toml")).unwrap();
    success(&run(
        root,
        &["edit", "review", "--set=package.version=1.22.6"],
    ));
    success(&run(root, &["gen", "review", "--offline"]));
    let target = area.join("recipe/SPECS/ed/ed.spec");
    let protected = [manifest, area.join(".config.toml"), area.join(".lock")];
    let saved = protected
        .iter()
        .chain(std::iter::once(&target))
        .map(|path| (path, fs::read(path).unwrap()))
        .collect::<Vec<_>>();
    #[cfg(unix)]
    let lock_identity = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(area.join(".lock")).unwrap().ino()
    };
    let assert_unchanged = || {
        for (path, bytes) in &saved {
            assert_eq!(fs::read(path).unwrap(), *bytes, "{}", path.display());
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(area.join(".lock")).unwrap().ino(),
                lock_identity
            );
        }
    };
    let reject = |output: &Path| {
        let result = command(root)
            .args([
                "edit",
                "review",
                "--set=package.version=1.22.9",
                "--apply",
                "--force",
                "--output",
            ])
            .arg(output)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert_unchanged();
    };
    for path in &protected {
        reject(path);
    }
    #[cfg(unix)]
    for path in &protected {
        for symbolic in [false, true] {
            let alias = root.join("protected-alias.spec");
            if symbolic {
                std::os::unix::fs::symlink(path, &alias).unwrap();
            } else {
                fs::hard_link(path, &alias).unwrap();
            }
            reject(&alias);
            fs::remove_file(alias).unwrap();
        }
    }
    let copy = root.join("review-copy.spec");
    let result = command(root)
        .args(["edit", "review", "--apply", "--output"])
        .arg(&copy)
        .output()
        .unwrap();
    success(&result);
    assert_file(copy, &version_spec("1.22.6"));
    assert_unchanged();
}

#[test]
fn invalid_names_and_interrupted_areas_fail_without_overwriting_files() {
    let directory = workspace();
    let root = directory.path();
    for args in [
        vec!["new", "../escaped"],
        vec!["new", "."],
        vec!["new", "demo", "--pkgname", "../ed"],
    ] {
        let output = run(root, &args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(!root.join("work").exists());
    }
    fs::create_dir_all(root.join("work/interrupted")).unwrap();
    fs::write(root.join("work/interrupted/notes"), "keep\n").unwrap();
    let output = run(root, &["new", "interrupted", "--force"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_file(root.join("work/interrupted/notes"), "keep\n");
    assert_eq!(
        fs::read_dir(root.join("work/interrupted")).unwrap().count(),
        1
    );
    let uninitialized = tempfile::tempdir().unwrap();
    let output = run(uninitialized.path(), &["new", "demo", "--pkgname", "demo"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(fs::read_dir(uninitialized.path()).unwrap().count(), 0);
    success(&run(uninitialized.path(), &["init"]));
    success(&run(uninitialized.path(), &["new", "demo"]));
    let nested = root.join("nested");
    fs::create_dir_all(nested.join(".ruyiconfig")).unwrap();
    fs::write(nested.join(".ruyiconfig/config.toml"), "invalid [toml").unwrap();
    let output = run(&nested, &["new", "demo", "--pkgname", "demo"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!root.join("work/demo").exists());
    assert!(!nested.join("work").exists());
}

#[cfg(unix)]
#[test]
fn managed_symlinks_are_rejected_without_writing_to_their_targets() {
    use std::os::unix::fs::symlink;
    let directory = workspace();
    let root = directory.path();
    let external = tempfile::tempdir().unwrap();
    fs::create_dir(root.join("work")).unwrap();
    symlink(external.path(), root.join("work/linked")).unwrap();
    assert_eq!(
        run(root, &["new", "linked", "--force"]).status.code(),
        Some(1)
    );
    assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    success(&run(root, &["new", "ed"]));
    let manifest = root.join("work/ed/ed.toml");
    fs::remove_file(&manifest).unwrap();
    fs::write(external.path().join("keep"), "keep\n").unwrap();
    symlink(external.path().join("keep"), &manifest).unwrap();
    assert_eq!(run(root, &["new", "ed", "--force"]).status.code(), Some(1));
    assert_file(external.path().join("keep"), "keep\n");
}

#[test]
fn author_is_snapshotted_at_init_and_workspace_edits_are_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let global = directory.path().join("gitconfig");
    fs::write(
        &global,
        "[user]\nname = Initial Author\nemail = initial@example.org\n",
    )
    .unwrap();
    success(
        &command(directory.path())
            .env("GIT_CONFIG_GLOBAL", &global)
            .args(["init", root.to_str().unwrap()])
            .output()
            .unwrap(),
    );
    fs::write(
        &global,
        "[user]\nname = Changed Author\nemail = changed@example.org\n",
    )
    .unwrap();
    // Reinitialization must not refresh user-owned defaults from Git or the environment.
    success(
        &command(&root)
            .env("GIT_CONFIG_GLOBAL", &global)
            .arg("init")
            .output()
            .unwrap(),
    );
    let output = command(&root)
        .env("GIT_CONFIG_GLOBAL", &global)
        .args(["new", "demo", "--stdout"])
        .output()
        .unwrap();
    assert_eq!(
        document(&output)["spec"]["contributors"][0].as_str(),
        Some("Initial Author <initial@example.org>")
    );
    let config_path = root.join(".ruyiconfig/config.toml");
    let mut config: toml::Table =
        toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    for author in [
        "Workspace Author <local@example.org>",
        "",
        "%{unsafe} <a@example.org>",
    ] {
        config.insert("author".into(), toml::Value::String(author.into()));
        fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
        let output = run(&root, &["new", "demo", "--stdout"]);
        let doc = document(&output);
        if author.starts_with("Workspace") {
            assert_eq!(doc["spec"]["contributors"][0].as_str(), Some(author));
        } else {
            assert!(doc["spec"]["contributors"].as_array().unwrap().is_empty());
            assert!(output_text(&output.stderr).contains(".ruyiconfig/config.toml"));
        }
    }
    let empty = directory.path().join("empty");
    fs::write(&global, "").unwrap();
    let output = command(directory.path())
        .env("GIT_CONFIG_GLOBAL", &global)
        .args(["init", empty.to_str().unwrap()])
        .output()
        .unwrap();
    success(&output);
    assert!(output_text(&output.stderr).contains("author is missing or invalid"));
    assert!(
        document(&run(&empty, &["new", "demo", "--stdout"]))["spec"]["contributors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn autotools_scaffolds_share_the_contract_and_feed_existing_generation() {
    let directory = workspace();
    let root = directory.path();
    let args = ["new", "ed", "--build-system", "autotools", "--stdout"];
    let standard = run(root, &args);
    let mut scaffold = document(&standard);
    assert!(scaffold["build"].get("stages").is_none());
    let fixture: toml::Value = toml::from_str(include_str!("../../examples/ed/ed.toml")).unwrap();
    for field in ["spec", "package", "sources"] {
        scaffold[field] = fixture[field].clone();
    }
    support::authoring_workspace(root, "ed", "ed", &toml::to_string(&scaffold).unwrap());
    let defaults = run(root, &["gen", "ed", "--stdout"]);
    success(&defaults);
    assert_eq!(
        output_text(&defaults.stdout),
        include_str!("../fixtures/ed.spec").replace("BuildRequires:  lzip\n", "")
    );
    // An explicit package list replaces defaults and includes ed's archive tool.
    scaffold["build-requires"] = fixture["build-requires"].clone();
    support::authoring_workspace(root, "ed", "ed", &toml::to_string(&scaffold).unwrap());
    let generated = run(root, &["gen", "ed", "--stdout"]);
    success(&generated);
    assert_eq!(
        output_text(&generated.stdout),
        include_str!("../fixtures/ed.spec")
    );

    // Optional guidance must describe fields that the generator already consumes.
    let source = toml::to_string(&scaffold).unwrap()
        + "\n[build.stages.conf]\noptions = [\"--enable-example\"]\nprepend = '''autoreconf -fiv\n'''\n";
    support::authoring_workspace(root, "ed", "ed", &source);
    let customized = run(root, &["gen", "ed", "--stdout"]);
    success(&customized);
    let spec = output_text(&customized.stdout);
    assert!(spec.contains("BuildOption(conf):  --enable-example"));
    assert!(spec.contains("%conf -p\nautoreconf -fiv"));

    scaffold["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    fs::write(
        root.join("work/ed/ed.toml"),
        toml::to_string(&scaffold).unwrap(),
    )
    .unwrap();
    let missing = run(root, &["gen", "ed"]);
    assert_eq!(missing.status.code(), Some(0));
    assert!(output_text(&missing.stderr).contains("RPK004"));
    assert!(!root.join("ed.spec").exists());
    for system in ["unknown", "", "../autotools"] {
        let invalid = run(root, &["new", "other", "--build-system", system]);
        assert_eq!(invalid.status.code(), Some(2));
        assert!(!root.join("other.toml").exists());
    }
}

#[test]
fn cmake_and_meson_scaffolds_render_without_fabricated_requirements() {
    let directory = workspace();
    let root = directory.path();
    let fixture: toml::Value = toml::from_str(include_str!("../../examples/ed/ed.toml")).unwrap();
    for system in ["cmake", "meson"] {
        let scaffold = run(
            root,
            &[
                "new",
                system,
                "--pkgname",
                system,
                "--build-system",
                system,
                "--stdout",
            ],
        );
        let mut manifest = document(&scaffold);
        for field in ["spec", "package", "sources"] {
            manifest[field] = fixture[field].clone();
        }
        manifest["package"]["name"] = system.into();
        support::authoring_workspace(root, system, system, &toml::to_string(&manifest).unwrap());
        let generated = run(root, &["gen", system, "--stdout"]);
        success(&generated);
        let spec = output_text(&generated.stdout);
        assert!(
            spec.contains(&format!("BuildSystem:    {system}")),
            "{system}"
        );
        assert!(
            !spec.contains("BuildRequires:"),
            "{system}: no requirements expected"
        );
    }
}

#[test]
fn gen_reports_every_unfilled_scaffold_field_at_once() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "demo", "--pkgname", "demo"]));
    let generated = run(&root.join("work/demo"), &["gen", "demo"]);
    assert_eq!(generated.status.code(), Some(1));
    assert!(generated.stdout.is_empty());
    // A blank scaffold must surface every field that still needs a value in one
    // report, not stop at the first table. package.vcs used to abort here during
    // deserialization and hide the rest.
    let report = output_text(&generated.stderr);
    for field in [
        "package.version",
        "package.summary",
        "package.license",
        "package.url",
        "package.description",
        "sources.0.url",
        "package.files",
    ] {
        assert!(
            report.contains(field),
            "missing {field} in report: {report}"
        );
    }
    assert!(!root.join("work/demo/demo.spec").exists());
}

#[test]
fn imported_authoring_preserves_bytes_binding_and_existing_gen() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success(&run(root, &["init"]));
    let source = include_str!("../../examples/ed/ed.toml");
    fs::write(root.join("upstream.toml"), source).unwrap();
    let preview = run(
        root,
        &["new", "review", "--from-toml=upstream.toml", "--stdout"],
    );
    success(&preview);
    assert_eq!(preview.stdout, source.as_bytes());
    assert!(!root.join("work/review").exists());
    success(&run(root, &["new", "review", "--from-toml=upstream.toml"]));
    assert_file(root.join("work/review/ed.toml"), source);
    assert_file(root.join("upstream.toml"), source);
    success(&run(root, &["gen", "review", "--offline", "--check"]));
    for (contents, args) in [
        (
            "broken = [",
            vec!["new", "rejected", "--from-toml=bad.toml"],
        ),
        (
            "[package]\nname = 1",
            vec!["new", "rejected", "--from-toml=bad.toml"],
        ),
        (
            source,
            vec!["new", "rejected", "--from-toml=bad.toml", "--pkgname=other"],
        ),
    ] {
        fs::write(root.join("bad.toml"), contents).unwrap();
        assert_eq!(run(root, &args).status.code(), Some(1));
        assert!(!root.join("work/rejected").exists());
    }
    fs::write(root.join("partial.toml"), "[package]\nname = 'ed'\n").unwrap();
    success(&run(root, &["new", "partial", "--from-toml=partial.toml"]));
    assert_file(
        root.join("work/partial/ed.toml"),
        "[package]\nname = 'ed'\n",
    );
    assert_eq!(
        run(root, &["gen", "partial", "--offline", "--check"])
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn local_scaffold_and_import_use_recipe_operations_without_a_git_repository() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success(&run(root, &["init"]));
    success(&run(root, &["new", "ed"]));
    let area = root.join("work/ed");
    fs::write(
        area.join("ed.toml"),
        include_str!("../../examples/ed/ed.toml"),
    )
    .unwrap();
    success(&run(root, &["gen", "ed", "--offline", "--apply"]));
    let spec = area.join("recipe/SPECS/ed/ed.spec");
    assert_file(&spec, include_str!("../fixtures/ed.spec"));
    success(&run(
        root,
        &["edit", "ed", "--set=package.version=1.22.6", "--apply"],
    ));
    assert_file(&spec, &version_spec("1.22.6"));
    success(&run(root, &["check", "ed"]));
    let preview = run(root, &["delete", "ed", "--dry-run", "--format=toml"]);
    success(&preview);
    let report: toml::Value = toml::from_str(output_text(&preview.stdout)).unwrap();
    assert!(
        report["authoring_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str() == fs::canonicalize(&spec).unwrap().to_str())
    );
    assert!(spec.exists());
    success(&run(root, &["delete", "ed", "--force", "--format=toml"]));
    assert!(!area.exists());
}

#[test]
fn spec_import_preserves_scripts_materials_and_uses_a_fixed_authoring_input() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success(&run(root, &["init"]));
    let input = root.join("input");
    fs::create_dir(&input).unwrap();
    let source = include_str!("../fixtures/ed.spec")
        .replace("%description", "%build\necho opaque-script\n\n%description");
    fs::write(input.join("ed.spec"), &source).unwrap();
    fs::write(input.join("fix.patch"), "patch bytes\n").unwrap();
    fs::write(input.join("files.list"), "/usr/bin/ed\n").unwrap();
    let preview = run(
        root,
        &[
            "new",
            "copy",
            "--from-dir=input",
            "--pkgname=ed",
            "--stdout",
        ],
    );
    let projected = document(&preview);
    assert_eq!(projected["package"]["version"].as_str(), Some("1.22.5"));
    assert!(!root.join("work/copy").exists());
    success(&run(
        root,
        &["new", "copy", "--from-dir=input", "--pkgname=ed"],
    ));
    let area = root.join("work/copy");
    let manifest = area.join("ed.toml");
    // Checking a fresh import needs no output directory and publishes nothing.
    success(&run(root, &["gen", "copy", "--offline", "--check"]));
    assert!(!area.join("stage").exists());
    assert!(!area.join("authoring/.cache/ed.resolved.toml").exists());
    let rejected = run(
        root,
        &[
            "gen",
            "copy",
            "--offline",
            "--output=missing/output.spec",
            "--format=toml",
        ],
    );
    assert_eq!(rejected.status.code(), Some(1));
    let report: toml::Table = toml::from_str(output_text(&rejected.stdout)).unwrap();
    assert_eq!(report["error"]["code"].as_str(), Some("output-target"));
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing/output.spec")
    );
    assert!(!root.join("missing").exists());
    assert_file(area.join("recipe/SPECS/ed/ed.spec"), &source);
    for name in ["fix.patch", "files.list"] {
        assert_eq!(
            fs::read(area.join("recipe/SPECS/ed").join(name)).unwrap(),
            fs::read(input.join(name)).unwrap()
        );
    }
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("1.22.5", "1.22.6"),
    )
    .unwrap();
    success(&run(
        root,
        &["edit", "copy", "--set=package.version=1.22.7"],
    ));
    let generated = run(root, &["gen", "copy", "--offline", "--stdout"]);
    success(&generated);
    assert_eq!(
        generated.stdout,
        source.replace("1.22.5", "1.22.7").as_bytes()
    );
    success(&run(root, &["gen", "copy", "--offline", "--apply"]));
    success(&run(root, &["gen", "copy", "--offline", "--check"]));
    assert_file(
        area.join("recipe/SPECS/ed/ed.spec"),
        &source.replace("1.22.5", "1.22.7"),
    );
    assert_file(input.join("ed.spec"), &source);
    assert_eq!(
        run(root, &["new", "copy", "--from-dir=input", "--pkgname=ed"])
            .status
            .code(),
        Some(1)
    );
    success(&run(
        root,
        &["new", "renamed", "--from-dir=input", "--pkgname=other"],
    ));
    // Directory binding does not rename the imported RPM package.
    let renamed = run(root, &["gen", "renamed", "--offline", "--stdout"]);
    success(&renamed);
    assert_eq!(renamed.stdout, source.as_bytes());
    assert_file(
        root.join("work/renamed/recipe/SPECS/other/ed.spec"),
        &source,
    );
    success(&run(root, &["gen", "renamed", "--offline", "--apply"]));
    assert_file(
        root.join("work/renamed/recipe/SPECS/other/ed.spec"),
        &source,
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("fix.patch", input.join("linked.patch")).unwrap();
        assert_eq!(
            run(
                root,
                &["new", "unsafe-copy", "--from-dir=input", "--pkgname=ed"]
            )
            .status
            .code(),
            Some(1)
        );
        assert!(!root.join("work/unsafe-copy").exists());
    }
}

#[cfg(unix)]
#[test]
fn authoring_editor_opens_the_bound_input_without_generating_a_spec() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success(&run(root, &["init"]));
    let input = root.join("input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("ed.spec"), include_str!("../fixtures/ed.spec")).unwrap();
    fs::write(
        input.join("ed.toml"),
        include_str!("../../examples/ed/ed.toml"),
    )
    .unwrap();
    for (work, extra, author) in [
        ("blank", vec![], "blank.toml"),
        ("imported", vec!["--from-toml=input/ed.toml"], "ed.toml"),
        (
            "recipe",
            vec!["--from-dir=input", "--pkgname=ed"],
            "ed.toml",
        ),
    ] {
        success(&run(root, &[vec!["new", work], extra].concat()));
        let area = root.join("work").join(work);
        let target = area.join(author);
        let before = fs::read(&target).unwrap();
        let output = run(
            root,
            &[
                "open",
                work,
                "--authoring",
                "--editor",
                "printf '%s' > opened-path",
            ],
        );
        success(&output);
        assert_eq!(
            fs::read_to_string(root.join("opened-path")).unwrap(),
            fs::canonicalize(&target).unwrap().to_str().unwrap()
        );
        assert_eq!(fs::read(&target).unwrap(), before);
        assert!(!area.join("stage").exists());
    }
    fs::remove_file(root.join("work/blank/blank.toml")).unwrap();
    assert_eq!(
        run(
            root,
            &[
                "open",
                "blank",
                "--authoring",
                "--editor",
                "touch should-not-run"
            ]
        )
        .status
        .code(),
        Some(1)
    );
    assert!(!root.join("should-not-run").exists());
}

fn seed_recipe(root: &Path, work: &str) {
    fs::copy(
        root.join("openruyi/SPECS/ed/ed.spec"),
        root.join("work").join(work).join("recipe/SPECS/ed/ed.spec"),
    )
    .unwrap();
}

#[test]
fn repository_import_reads_main_and_keeps_local_changes_independent() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    git(&repo, &["switch", "--quiet", "-c", "other"]);
    fs::write(repo.join("SPECS/ed/ed.spec"), version_spec("9.0")).unwrap();
    git(&repo, &["commit", "-qam", "Other branch"]);
    assert_eq!(edit_version(root, "ed"), "1.22.5");
    let area = root.join("work/ed");
    assert!(!area.join("recipe").exists());
    success(&run(
        root,
        &["edit", "ed", "--set=package.version=1.22.6", "--apply"],
    ));
    fs::rename(&repo, root.join("offline-repo")).unwrap();
    assert_eq!(edit_version(root, "ed"), "1.22.6");
    assert_file(
        area.join("recipe/SPECS/ed/ed.spec"),
        &version_spec("1.22.6"),
    );
}

#[test]
fn directory_and_spec_imports_bind_paths_without_rewriting_names() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success(&run(root, &["init"]));
    fs::create_dir(root.join("renamed-dir")).unwrap();
    fs::write(
        root.join("renamed-dir/original.spec"),
        include_str!("../fixtures/ed.spec"),
    )
    .unwrap();
    fs::write(root.join("renamed-dir/fix.patch"), "patch material").unwrap();
    success(&run(root, &["new", "whole", "--from-dir=renamed-dir"]));
    assert_file(
        root.join("work/whole/recipe/SPECS/original/original.spec"),
        include_str!("../fixtures/ed.spec"),
    );
    assert_file(
        root.join("work/whole/recipe/SPECS/original/fix.patch"),
        "patch material",
    );
    fs::create_dir(root.join("empty")).unwrap();
    fs::write(
        root.join("renamed-dir/renamed-dir.spec"),
        include_str!("../fixtures/ed.spec"),
    )
    .unwrap();
    for (work, source) in [("empty-input", "empty"), ("ambiguous", "renamed-dir")] {
        let rejected = run(root, &["new", work, "--from-dir", source]);
        assert_eq!(rejected.status.code(), Some(1));
        assert!(!root.join("work").join(work).exists());
    }
    success(&run(
        root,
        &[
            "new",
            "explicit",
            "--from-spec=renamed-dir/original.spec",
            "--pkgname=renamed",
        ],
    ));
    assert_file(
        root.join("work/explicit/recipe/SPECS/renamed/original.spec"),
        include_str!("../fixtures/ed.spec"),
    );
    for (work, name) in [
        ("macro-name", "%global upstream ed\nName: %{upstream}"),
        (
            "conditional-name",
            "%if 0\nName: ed-bootstrap\n%else\nName: ed\n%endif",
        ),
    ] {
        let source = include_str!("../fixtures/ed.spec").replace("Name:           ed", name);
        fs::write(root.join("recipe.spec"), &source).unwrap();
        success(&run(root, &["new", work, "--from-spec=recipe.spec"]));
        assert_file(
            root.join(format!("work/{work}/recipe/SPECS/recipe/recipe.spec")),
            &source,
        );
        let generated = run(root, &["gen", work, "--offline", "--stdout"]);
        success(&generated);
        assert_eq!(generated.stdout, source.as_bytes());
    }
    let rejected = run(root, &["new", "recursive", "--from-dir=.", "--pkgname=ed"]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(!root.join("work/recursive").exists());
}
