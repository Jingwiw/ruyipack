// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Small process and output helpers shared by CLI integration tests.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use toml::Value;

pub fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ruyipack"))
}

/// Hermetic fixture commands never inherit the developer's Git identity or hooks.
pub fn isolated_command(program: impl AsRef<std::ffi::OsStr>, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(directory);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

pub fn git(directory: &Path, args: &[&str]) -> Output {
    let output = isolated_command("git", directory)
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    success(&output);
    output
}

pub fn output_text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("command output is UTF-8")
}

/// Parse a complete machine response; individual cases check its business values.
pub fn machine_report(output: &Output) -> Value {
    let stdout = output_text(&output.stdout);
    assert!(stdout.ends_with('\n'), "stdout has no trailing newline");
    toml::from_str(stdout).expect("machine report is one complete TOML document")
}

pub fn run(directory: &Path, args: &[&str]) -> Output {
    command()
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run ruyipack")
}

#[track_caller]
pub fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

#[track_caller]
pub fn quiet_success(output: &Output) {
    success(output);
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[track_caller]
pub fn rejected(output: &Output, message: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output_text(&output.stderr).contains(message), "{output:?}");
}

#[track_caller]
pub fn assert_file(path: impl AsRef<Path>, expected: &str) {
    let path = path.as_ref();
    let actual =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    assert_eq!(actual, expected, "{}", path.display());
}

/// An explicit existing WORK fixture, independent of `new` and any Git recipe.
pub fn authoring_workspace(
    root: &Path,
    work: &str,
    package: &str,
    source: &str,
) -> std::path::PathBuf {
    if !root.join(".ruyiconfig").exists() {
        success(&run(root, &["init"]));
    }
    let directory = root.join("work").join(work);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(".config.toml"),
        format!("pkg = {package:?}\ninput = \"authoring\"\n"),
    )
    .unwrap();
    fs::write(directory.join(format!("{package}.toml")), source).unwrap();
    directory
}

/// A committed recipe and existing WORK binding for source-bound stage tests.
pub fn recipe_workspace(
    root: &Path,
    work: &str,
    package: &str,
    source: &str,
) -> std::path::PathBuf {
    let development = authoring_workspace(root, work, package, "invalid authoring TOML!\n");
    let recipes = root.join("openruyi");
    fs::create_dir_all(recipes.join("SPECS").join(package)).unwrap();
    fs::write(
        recipes
            .join("SPECS")
            .join(package)
            .join(format!("{package}.spec")),
        source,
    )
    .unwrap();
    for args in [
        vec!["init", "--initial-branch=main", "--quiet"],
        vec!["add", "."],
        vec!["commit", "--quiet", "-m", "Fixture recipe baseline"],
    ] {
        git(&recipes, &args);
    }
    development
}
