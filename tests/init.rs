// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Scaffold contents, local directory checks, identity, and the existing gen consumer.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

#[allow(
    dead_code,
    reason = "The shared helper also supports JSON report tests."
)]
mod support;
use support::output_text;

fn command(directory: &Path) -> Command {
    let mut command = support::command();
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", directory)
        .env("TZ", "UTC")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Packager \"A\" \\测试")
        .env("GIT_AUTHOR_EMAIL", "packager@example.org")
        .current_dir(directory);
    command
}

fn run(directory: &Path, args: &[&str]) -> Output {
    command(directory).args(args).output().unwrap()
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn document(output: &Output) -> toml::Value {
    success(output);
    toml::from_str(output_text(&output.stdout)).unwrap()
}

#[test]
fn comment_modes_share_fields_and_filled_scaffolds_use_gen() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let standard = run(root, &["init", "ed", "--stdout"]);
    let full = run(root, &["init", "ed", "--stdout", "--comments", "full"]);
    let mut scaffold = document(&standard);
    assert_eq!(scaffold, document(&full));
    assert!(full.stdout.len() > standard.stdout.len());
    assert!(output_text(&standard.stderr).contains("name availability was not checked"));
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
    assert_eq!(scaffold["sources"]["0"]["sha256"].as_str(), Some(""));
    assert!(scaffold["package"]["vcs"].as_table().unwrap().is_empty());
    assert!(scaffold.get("build").is_none());
    assert!(!output_text(&standard.stdout).contains("SPDX-FileCopyrightText"));
    assert!(!output_text(&full.stdout).contains("RuyiPack"));
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    fs::write(root.join("ed.toml"), &standard.stdout).unwrap();
    let incomplete = run(root, &["gen", "ed"]);
    assert_eq!(incomplete.status.code(), Some(1));
    assert!(incomplete.stdout.is_empty());
    assert!(!root.join("ed.spec").exists());

    let filled: toml::Value = toml::from_str(include_str!("../examples/ed/ed.toml")).unwrap();
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
    fs::write(root.join("ed.toml"), toml::to_string(&scaffold).unwrap()).unwrap();
    let generated = command(root)
        .env("GIT_AUTHOR_NAME", "Different author")
        .args(["gen", "ed", "--stdout"])
        .output()
        .unwrap();
    success(&generated);
    assert_eq!(
        output_text(&generated.stdout),
        include_str!("fixtures/ed.spec")
    );
}

#[test]
fn local_entries_override_repository_history_and_force() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join("SPECS")).unwrap();
    fs::create_dir(root.join("SPECS/new-package")).unwrap();
    fs::create_dir(root.join("work")).unwrap();
    // No Git repository or published RPM exists; the working directory is enough.
    for action in ["--stdout", "--force", "--skip-existing"] {
        let output = run(&root.join("work"), &["init", "new-package", action]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(output_text(&output.stderr).contains("SPECS/new-package already exists"));
    }
    fs::create_dir(root.join("other-specs")).unwrap();
    let selected = run(
        &root.join("work"),
        &["init", "new-package", "--specs-dir", "../other-specs"],
    );
    success(&selected);
    assert!(!output_text(&selected.stderr).contains("was not checked"));
    assert!(root.join("work/new-package.toml").is_file());
}

#[test]
fn invalid_locations_and_names_fail_without_creating_directories() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("not-a-directory"), "original").unwrap();
    for args in [
        vec!["init", "demo", "--dir", "missing"],
        vec!["init", "demo", "--dir", "not-a-directory"],
        vec!["init", "demo", "--specs-dir", "missing"],
        vec!["init", "demo", "--specs-dir", "not-a-directory"],
        vec!["init", "../escaped"],
        vec!["init", "."],
    ] {
        let output = run(root, &args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }
    fs::write(root.join("SPECS"), "not a directory").unwrap();
    let invalid = run(root, &["init", "demo"]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(output_text(&invalid.stderr).contains("not a directory"));
    assert!(!root.join("demo.toml").exists());
}

#[test]
fn init_reuses_output_conflicts_without_exposing_a_second_path_option() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join("output")).unwrap();
    let args = ["init", "demo", "--dir", "output"];
    success(&run(root, &args));
    let target = root.join("output/demo.toml");
    let expected = fs::read(&target).unwrap();
    fs::write(&target, "# manual content\n").unwrap();
    let conflict = run(root, &args);
    assert_eq!(conflict.status.code(), Some(1));
    // The conflict help must offer only the options init actually accepts.
    let help = output_text(&conflict.stderr);
    assert!(
        help.contains("already exists with different content"),
        "{help}"
    );
    assert!(help.contains("--force"), "{help}");
    assert!(!help.contains("--output"), "{help}");
    for action in ["--stdout", "--diff", "--skip-existing"] {
        let output = run(root, &["init", "demo", "--dir", "output", action]);
        success(&output);
        assert_eq!(fs::read_to_string(&target).unwrap(), "# manual content\n");
    }
    success(&run(root, &["init", "demo", "--dir", "output", "--force"]));
    assert_eq!(fs::read(&target).unwrap(), expected);
    assert_eq!(
        run(root, &["init", "demo", "-o", "other.toml"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        run(root, &["init", "demo", "--stdout", "--force"])
            .status
            .code(),
        Some(2)
    );
    assert!(!root.join("other.toml").exists());
}

#[test]
fn author_uses_git_precedence_and_never_invents_a_missing_identity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    for (key, value) in [
        ("user.name", "Local Author"),
        ("user.email", "local@example.org"),
    ] {
        assert!(
            Command::new("git")
                .args(["config", key, value])
                .current_dir(root)
                .status()
                .unwrap()
                .success()
        );
    }
    let global = root.join("gitconfig");
    fs::write(
        &global,
        "[user]\nname = Global Author\nemail = global@example.org\n",
    )
    .unwrap();
    let local = command(root)
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env("GIT_CONFIG_GLOBAL", &global)
        .args(["init", "demo", "--stdout"])
        .output()
        .unwrap();
    assert_eq!(
        document(&local)["spec"]["contributors"][0].as_str(),
        Some("Local Author <local@example.org>")
    );
    let environment = run(root, &["init", "demo", "--stdout"]);
    assert_eq!(
        document(&environment)["spec"]["contributors"][0].as_str(),
        Some("Packager \"A\" \\测试 <packager@example.org>")
    );
    for key in ["user.name", "user.email"] {
        assert!(
            Command::new("git")
                .args(["config", "--unset", key])
                .current_dir(root)
                .status()
                .unwrap()
                .success()
        );
    }
    let from_global = command(root)
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env("GIT_CONFIG_GLOBAL", &global)
        .args(["init", "demo", "--stdout"])
        .output()
        .unwrap();
    assert_eq!(
        document(&from_global)["spec"]["contributors"][0].as_str(),
        Some("Global Author <global@example.org>")
    );
    for name in [None, Some("%{unsafe}")] {
        let mut cmd = command(root);
        cmd.env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL");
        if let Some(name) = name {
            cmd.env("GIT_AUTHOR_NAME", name)
                .env("GIT_AUTHOR_EMAIL", "author@example.org");
        }
        let output = cmd.args(["init", "demo", "--stdout"]).output().unwrap();
        assert!(
            document(&output)["spec"]["contributors"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(output_text(&output.stderr).contains("fill spec.contributors"));
    }
}
