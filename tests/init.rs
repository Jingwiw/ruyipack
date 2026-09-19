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
    assert!(scaffold.get("subpackages").is_none());
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
fn full_comments_offer_optional_subpackages_that_feed_generation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let full = run(root, &["init", "ed", "--stdout", "--comments", "full"]);
    success(&full);

    // Consume the real commented example, allowing blank comments and spacing.
    let example = output_text(&full.stdout)
        .lines()
        .map(str::trim_start)
        .skip_while(|line| {
            line.strip_prefix('#').map(str::trim_start) != Some("[subpackages.devel]")
        })
        .take_while(|line| line.is_empty() || line.starts_with('#'))
        .map(|line| line.strip_prefix('#').unwrap_or_default().trim_start())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!example.is_empty(), "missing commented subpackage example");
    let source = include_str!("../examples/ed/ed.toml").to_owned() + "\n" + &example;
    fs::write(root.join("ed.toml"), source).unwrap();
    let generated = run(root, &["gen", "ed", "--stdout"]);
    success(&generated);
    let spec = output_text(&generated.stdout);
    for section in ["%package", "%description", "%files"] {
        assert!(
            spec.lines()
                .any(|line| line.split_whitespace().eq([section, "devel"])),
            "missing {section} devel: {spec}"
        );
    }
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

#[test]
fn autotools_scaffolds_share_the_contract_and_feed_existing_generation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let args = ["init", "ed", "--build-system", "autotools", "--stdout"];
    let standard = run(root, &args);
    let full = run(
        root,
        &[
            "init",
            "ed",
            "--build-system",
            "autotools",
            "--comments",
            "full",
            "--stdout",
        ],
    );
    let mut scaffold = document(&standard);
    assert_eq!(scaffold, document(&full));
    assert!(full.stdout.len() > standard.stdout.len());
    // autoreconf is autotools-specific and not every package needs it, so the
    // contract carries it as a full-only note rather than a standard example.
    assert!(output_text(&full.stdout).contains("autoreconf -fiv"));
    assert!(!output_text(&standard.stdout).contains("autoreconf"));
    let contract: toml::Value = toml::from_str(include_str!(
        "../profiles/openruyi-v1/buildsystems/autotools.toml"
    ))
    .unwrap();
    assert_eq!(scaffold["build"]["system"], contract["name"]);
    assert_eq!(
        scaffold["build-requires"]["rpm"],
        contract["build-requires"]
    );
    assert!(scaffold["build"].get("stages").is_none());
    let fixture: toml::Value = toml::from_str(include_str!("../examples/ed/ed.toml")).unwrap();
    for field in ["spec", "package", "sources"] {
        scaffold[field] = fixture[field].clone();
    }
    // Keep the actual template's build configuration and add only ed's archive tool.
    scaffold["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .push("lzip".into());
    fs::write(root.join("ed.toml"), toml::to_string(&scaffold).unwrap()).unwrap();
    let generated = run(root, &["gen", "ed", "--stdout"]);
    success(&generated);
    assert_eq!(
        output_text(&generated.stdout),
        include_str!("fixtures/ed.spec")
    );

    // Optional guidance must describe fields that the generator already consumes.
    let source = toml::to_string(&scaffold).unwrap()
        + "\n[build.stages.conf]\noptions = [\"--enable-example\"]\nprepend = '''autoreconf -fiv\n'''\n";
    fs::write(root.join("ed.toml"), source).unwrap();
    let customized = run(root, &["gen", "ed", "--stdout"]);
    success(&customized);
    let spec = output_text(&customized.stdout);
    assert!(spec.contains("BuildOption(conf):  --enable-example"));
    assert!(spec.contains("%conf -p\nautoreconf -fiv"));

    scaffold["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    fs::write(root.join("ed.toml"), toml::to_string(&scaffold).unwrap()).unwrap();
    let missing = run(root, &["gen", "ed"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(output_text(&missing.stderr).contains("RPK004"));
    assert!(!root.join("ed.spec").exists());
    for system in ["unknown", "", "../autotools"] {
        let invalid = run(root, &["init", "other", "--build-system", system]);
        assert_eq!(invalid.status.code(), Some(2));
        assert!(!root.join("other.toml").exists());
    }
}

#[test]
fn cmake_and_meson_scaffolds_render_without_fabricated_requirements() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    // Each system pairs its contract file with the stage actions openRuyi ships.
    let cases = [
        (
            "cmake",
            include_str!("../profiles/openruyi-v1/buildsystems/cmake.toml"),
            ["%cmake", "%cmake_build", "%cmake_install", "%ctest"],
        ),
        (
            "meson",
            include_str!("../profiles/openruyi-v1/buildsystems/meson.toml"),
            ["%meson", "%meson_build", "%meson_install", "%meson_test"],
        ),
    ];
    for (system, contract_toml, actions) in cases {
        let scaffold = run(
            root,
            &["init", system, "--build-system", system, "--stdout"],
        );
        let document = document(&scaffold);
        let contract: toml::Value = toml::from_str(contract_toml).unwrap();
        assert_eq!(document["build"]["system"], contract["name"]);
        // openRuyi mandates no build tools for these systems, so nothing is prefilled.
        assert_eq!(
            document["build-requires"]["rpm"],
            contract["build-requires"]
        );
        assert!(
            document["build-requires"]["rpm"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let text = output_text(&scaffold.stdout);
        for action in actions {
            assert!(text.contains(action), "{system}: missing {action}");
        }

        // A filled manifest with no build requirements must generate cleanly and
        // must not raise RPK004: the contract declares nothing to enforce.
        let manifest = format!(
            "[spec]\ncopyright-years = \"2026\"\ncontributors = [\"P <p@example.org>\"]\n\
             [package]\nname = \"{system}\"\nversion = \"1.0\"\nsummary = \"A {system} sample\"\n\
             license = \"MIT\"\nurl = \"https://example.org/{system}\"\n\
             description = \"A sample used to check {system} rendering.\"\n\
             [package.vcs]\nno-public-repository = true\n\
             [sources.0]\nurl = \"https://example.org/{system}-1.0.tar.gz\"\n\
             sha256 = \"{zeros}\"\n[build]\nsystem = \"{system}\"\n\
             [build-requires]\nrpm = []\n[package.files]\nentries = [\"%{{_bindir}}/{system}\"]\n",
            system = system,
            zeros = "0".repeat(64),
        );
        fs::write(root.join(format!("{system}.toml")), manifest).unwrap();
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
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success(&run(root, &["init", "demo"]));
    let generated = run(root, &["gen", "demo"]);
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
        "package.vcs",
        "sources.0.url",
        "sources.0.sha256",
        "package.files",
    ] {
        assert!(
            report.contains(field),
            "missing {field} in report: {report}"
        );
    }
    assert!(!root.join("demo.spec").exists());
}
