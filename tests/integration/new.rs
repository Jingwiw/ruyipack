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
    let repo = root.join("openruyi");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    for (path, contents) in [
        ("SPECS/ed/ed.spec", include_str!("../fixtures/ed.spec")),
        ("SPECS/other/other.spec", "other package\n"),
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

fn binding(area: &Path) -> toml::Value {
    toml::from_str(&fs::read_to_string(area.join(".config.toml")).unwrap()).unwrap()
}

#[test]
fn edit_work_reads_its_saved_package_checkout_and_only_publishes_there() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    for work in ["ed-test", "ed-other"] {
        success(&run(root, &["new", work, "--pkgname", "ed"]));
    }
    let area = root.join("work/ed-test");
    let target = area.join("checkout/SPECS/ed/ed.spec");
    let original = version_spec("1.22.6");
    fs::write(&target, &original).unwrap();
    let retained: Vec<_> = [
        repo.join("SPECS/ed/ed.spec"),
        area.join("ed.toml"),
        root.join("work/ed-other/checkout/SPECS/ed/ed.spec"),
        root.join("work/ed-other/ed.toml"),
        root.join("work/ed-other/.config.toml"),
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(&path).unwrap();
        (path, bytes)
    })
    .collect();
    let worktrees = git(&repo, &["worktree", "list", "--porcelain"]).stdout;
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
    assert_eq!(binding(&area)["input"].as_str(), Some("edit"));
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
    assert_eq!(
        git(&repo, &["worktree", "list", "--porcelain"]).stdout,
        worktrees
    );
}

#[test]
fn edit_work_wins_over_a_bare_file_and_explicit_paths_still_select_files() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "ed-test", "--pkgname", "ed"]));
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
        root.join("work/ed-test/checkout/SPECS/ed/ed.spec"),
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
    let target = area.join("checkout/SPECS/ed/ed.spec");
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
    assert!(area.join("checkout/.git").is_file());
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
fn first_work_uses_committed_main_and_existing_work_keeps_its_manual_branch() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    let main = git(&repo, &["rev-parse", "refs/heads/main"]).stdout;
    git(&repo, &["switch", "--quiet", "-c", "recipe-topic"]);
    fs::write(repo.join("SPECS/ed/ed.spec"), version_spec("9.0")).unwrap();
    git(
        &repo,
        &["commit", "--quiet", "-am", "Different recipe HEAD"],
    );
    success(&run(root, &["new", "new-from-main", "--pkgname", "ed"]));
    let view = run(
        root,
        &[
            "inspect",
            "edit-from-main",
            "--pkgname",
            "ed",
            "--field",
            "package.version",
            "--editable",
        ],
    );
    assert_eq!(
        document(&view)["package"]["version"].as_str(),
        Some("1.22.5")
    );
    let area = root.join("work/edit-from-main");
    let saved_binding = fs::read(area.join(".config.toml")).unwrap();
    assert!(!area.join("checkout").exists());
    assert_eq!(fs::read(area.join(".config.toml")).unwrap(), saved_binding);
    let readonly_worktrees = git(&repo, &["worktree", "list", "--porcelain"]).stdout;
    fs::write(repo.join("SPECS/ed/ed.spec"), version_spec("9.1")).unwrap();
    assert_eq!(edit_version(root, "edit-from-main"), "1.22.5");
    assert!(!area.join("checkout").exists());
    assert_eq!(
        git(&repo, &["worktree", "list", "--porcelain"]).stdout,
        readonly_worktrees
    );
    git(&repo, &["restore", "SPECS/ed/ed.spec"]);
    success(&run(
        root,
        &[
            "edit",
            "edit-from-main",
            "--set",
            "package.version=1.22.6",
            "--apply",
        ],
    ));
    assert_eq!(binding(&area)["pkg"].as_str(), Some("ed"));
    assert_eq!(binding(&area)["input"].as_str(), Some("edit"));
    for (work, version) in [("new-from-main", "1.22.5"), ("edit-from-main", "1.22.6")] {
        let checkout = root.join("work").join(work).join("checkout");
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]).stdout, main);
        assert_file(checkout.join("SPECS/ed/ed.spec"), &version_spec(version));
    }

    let checkout = root.join("work/new-from-main/checkout");
    git(&checkout, &["switch", "--quiet", "-c", "manual-work"]);
    fs::write(checkout.join("SPECS/ed/ed.spec"), version_spec("2.0")).unwrap();
    let branch = git(&checkout, &["symbolic-ref", "HEAD"]).stdout;
    success(&run(root, &["new", "new-from-main", "--skip-existing"]));
    assert_eq!(edit_version(root, "new-from-main"), "2.0");
    success(&run(
        root,
        &[
            "edit",
            "new-from-main",
            "--set",
            "package.version=2.1",
            "--apply",
        ],
    ));
    assert_eq!(git(&checkout, &["symbolic-ref", "HEAD"]).stdout, branch);
    assert_file(checkout.join("SPECS/ed/ed.spec"), &version_spec("2.1"));
    assert_file(repo.join("SPECS/ed/ed.spec"), &version_spec("9.0"));

    git(&repo, &["branch", "-D", "main"]);
    let worktrees = git(&repo, &["worktree", "list", "--porcelain"]).stdout;
    for args in [
        vec!["new", "missing-main-new", "--pkgname", "ed"],
        vec![
            "inspect",
            "missing-main-edit",
            "--pkgname",
            "ed",
            "--field",
            "package.version",
            "--editable",
        ],
    ] {
        let output = run(root, &args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(!root.join("work").join(args[1]).exists());
        assert_eq!(
            git(&repo, &["worktree", "list", "--porcelain"]).stdout,
            worktrees
        );
    }
}

#[test]
fn edit_missing_or_misnamed_package_spec_is_rejected_before_creating_a_worktree() {
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
    let worktrees = git(&repo, &["worktree", "list", "--porcelain"]).stdout;
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
            assert_eq!(
                git(&repo, &["worktree", "list", "--porcelain"]).stdout,
                worktrees
            );
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
    let dotted_worktrees = git(&repo, &["worktree", "list", "--porcelain"]).stdout;
    assert_eq!(edit_version(root, "ed.plus"), "1.22.5");
    let area = root.join("work/ed.plus");
    assert_file(area.join(".config.toml"), "pkg = \"ed.plus\"\n");
    assert!(!area.join("checkout").exists());
    assert_eq!(
        git(&repo, &["worktree", "list", "--porcelain"]).stdout,
        dotted_worktrees
    );
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
    assert!(area.join("checkout/SPECS/ed.plus/ed.plus.spec").is_file());
    assert_eq!(edit_version(root, "ed.plus"), "1.22.6");
}

#[test]
fn edit_work_refuses_a_locked_binding_even_for_read_only_operations() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "ed-test", "--pkgname", "ed"]));
    let area = root.join("work/ed-test");
    let target = area.join("checkout/SPECS/ed/ed.spec");
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
    drop(lock);
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
fn package_binding_creates_one_sparse_checkout_and_previews_write_nothing() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    let baseline = git(&repo, &["rev-parse", "HEAD"]).stdout;
    let worktrees = git(&repo, &["worktree", "list", "--porcelain"]).stdout;
    for action in ["--stdout", "--diff"] {
        let preview = run(root, &["new", "ed-test", "--pkgname", "ed", action]);
        success(&preview);
        assert!(!root.join("work").exists());
        assert_eq!(
            git(&repo, &["worktree", "list", "--porcelain"]).stdout,
            worktrees
        );
    }
    fs::write(
        repo.join("SPECS/other/other.spec"),
        "unrelated local edit\n",
    )
    .unwrap();
    let output = run(
        &repo.join("scripts/nested"),
        &["new", "ed-test", "--pkgname", "ed"],
    );
    success(&output);
    let area = root.join("work/ed-test");
    let checkout = area.join("checkout");
    assert!(checkout.join(".git").is_file());
    assert!(area.join(".config.toml").is_file());
    let manifest_text = fs::read_to_string(area.join("ed.toml")).unwrap();
    assert!(manifest_text.contains("existing SPEC contents are not imported"));
    let manifest: toml::Value = toml::from_str(&manifest_text).unwrap();
    assert_eq!(manifest["package"]["name"].as_str(), Some("ed"));
    assert_eq!(
        document(&run(root, &["new", "ed-test", "--stdout"]))["package"]["name"].as_str(),
        Some("ed")
    );
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]).stdout, baseline);
    for path in [
        "SPECS/ed/ed.spec",
        "SPECS/README",
        "scripts/nested/helper",
        "policies/rules",
        "README",
    ] {
        assert_eq!(
            fs::read(checkout.join(path)).unwrap(),
            fs::read(repo.join(path)).unwrap(),
            "{path}"
        );
    }
    assert!(!checkout.join("SPECS/other").exists());
    assert_file(
        repo.join("SPECS/other/other.spec"),
        "unrelated local edit\n",
    );
    fs::write(
        area.join("ed.toml"),
        include_str!("../../examples/ed/ed.toml"),
    )
    .unwrap();
    fs::write(checkout.join("SPECS/ed/ed.spec"), version_spec("8.0")).unwrap();
    success(&run(
        &repo.join("scripts/nested"),
        &["gen", "ed-test", "--offline"],
    ));
    assert_file(checkout.join("SPECS/ed/ed.spec"), &version_spec("8.0"));
    assert_file(
        area.join("stage/ed.candidate.spec"),
        include_str!("../fixtures/ed.spec"),
    );
    assert!(area.join("ed.resolved.toml").is_file());
    success(&run(
        &repo.join("scripts/nested"),
        &["gen", "ed-test", "--offline", "--spec=auto", "--force"],
    ));
    assert_file(
        checkout.join("SPECS/ed/ed.spec"),
        include_str!("../fixtures/ed.spec"),
    );
    assert!(!root.join("ed-test.spec").exists());
    assert!(!area.join("ed-test.toml").exists());
}

#[cfg(unix)]
#[test]
fn checkout_filter_cannot_publish_a_stale_authoring_input() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    // A trusted checkout filter models an external authoring edit while Git
    // materializes files: the candidate must not publish the older manifest.
    fs::write(
        repo.join(".gitattributes"),
        "README filter=manifest-change\n",
    )
    .unwrap();
    git(&repo, &["add", ".gitattributes"]);
    git(
        &repo,
        &["commit", "--quiet", "-m", "Checkout filter fixture"],
    );
    success(&run(root, &["inspect", "ed-race", "--pkgname", "ed"]));
    let race = root.join("work/ed-race");
    let manifest = race.join("ed.toml");
    let input = include_str!("../../examples/ed/ed.toml")
        .replace("version = \"1.22.5\"", "version = \"1.22.6\"");
    fs::write(&manifest, &input).unwrap();
    let binding = fs::read(race.join(".config.toml")).unwrap();
    let filter = root.join("change-manifest.sh");
    let quoted_manifest = shell_words::quote(manifest.to_str().unwrap());
    fs::write(&filter, format!(
        "sed 's/version = \"1.22.6\"/version = \"2\"/' {quoted_manifest} > {quoted_manifest}.updated\n\
         mv {quoted_manifest}.updated {quoted_manifest}\ncat\n"
    )).unwrap();
    let smudge = format!("sh {}", shell_words::quote(filter.to_str().unwrap()));
    git(&repo, &["config", "filter.manifest-change.smudge", &smudge]);
    git(&repo, &["config", "filter.manifest-change.clean", "cat"]);
    git(
        &repo,
        &["config", "filter.manifest-change.required", "true"],
    );
    let changed = run(
        root,
        &["gen", "ed-race", "--offline", "--spec=auto", "--force"],
    );
    assert_eq!(changed.status.code(), Some(1), "{changed:?}");
    assert!(
        output_text(&changed.stderr).contains("changed"),
        "{changed:?}"
    );
    assert!(changed.stdout.is_empty());
    assert_file(
        &manifest,
        &input.replace("version = \"1.22.6\"", "version = \"2\""),
    );
    assert_eq!(fs::read(race.join(".config.toml")).unwrap(), binding);
    assert_file(
        race.join("checkout/SPECS/ed/ed.spec"),
        include_str!("../fixtures/ed.spec"),
    );
}

#[test]
fn nested_package_selection_preserves_source_sparse_settings_and_manual_branches() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    fs::create_dir(repo.join("distro")).unwrap();
    fs::rename(repo.join("SPECS"), repo.join("distro/recipes")).unwrap();
    fs::create_dir(repo.join("distro/tools")).unwrap();
    fs::write(repo.join("distro/tools/helper"), "sibling helper\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "Nested recipes"]);
    fs::write(
        root.join(".ruyiconfig/config.toml"),
        "recipes = 'openruyi'\nwork = 'work'\nspecs = 'distro/./recipes'\n",
    )
    .unwrap();
    git(&repo, &["sparse-checkout", "set", "scripts"]);
    assert!(!repo.join("distro/recipes/ed").exists());
    let sparse = fs::read(repo.join(".git/info/sparse-checkout")).unwrap();
    let status = git(&repo, &["status", "--porcelain=v1"]).stdout;
    success(&run(root, &["new", "nested-ed", "--pkgname", "ed"]));
    let checkout = root.join("work/nested-ed/checkout");
    assert_file(
        checkout.join("distro/recipes/ed/ed.spec"),
        include_str!("../fixtures/ed.spec"),
    );
    assert_file(checkout.join("distro/tools/helper"), "sibling helper\n");
    assert!(!checkout.join("distro/recipes/other").exists());
    git(&checkout, &["switch", "--quiet", "-c", "manual-topic"]);
    let branch = git(&checkout, &["symbolic-ref", "HEAD"]).stdout;
    success(&run(root, &["new", "nested-ed", "--skip-existing"]));
    assert_eq!(git(&checkout, &["symbolic-ref", "HEAD"]).stdout, branch);
    assert_eq!(
        fs::read(repo.join(".git/info/sparse-checkout")).unwrap(),
        sparse
    );
    assert_eq!(git(&repo, &["status", "--porcelain=v1"]).stdout, status);
    assert!(!repo.join("distro/recipes/ed").exists());
}

#[test]
fn selected_uncommitted_materials_are_rejected_before_creating_a_worktree() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    let worktrees = git(&repo, &["worktree", "list", "--porcelain"]).stdout;
    let path = repo.join("SPECS/ed/ed.spec");
    fs::write(&path, "uncommitted edit\n").unwrap();
    success(&run(
        root,
        &["new", "ed-test", "--pkgname", "ed", "--stdout"],
    ));
    assert!(!root.join("work").exists());
    let output = run(root, &["new", "ed-test", "--pkgname", "ed", "--force"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_file(&path, "uncommitted edit\n");
    assert!(root.join("work/ed-test/.config.toml").is_file());
    assert!(!root.join("work/ed-test/checkout").exists());
    git(&repo, &["add", "SPECS/ed/ed.spec"]);
    assert_eq!(run(root, &["new", "ed"]).status.code(), Some(1));
    git(
        &repo,
        &["restore", "--staged", "--worktree", "SPECS/ed/ed.spec"],
    );
    fs::write(repo.join("SPECS/ed/new.patch"), "untracked patch\n").unwrap();
    assert_eq!(run(root, &["new", "ed"]).status.code(), Some(1));
    fs::remove_file(repo.join("SPECS/ed/new.patch")).unwrap();
    fs::write(repo.join(".git/info/exclude"), "SPECS/ed/ignored.tar.gz\n").unwrap();
    fs::write(
        repo.join("SPECS/ed/ignored.tar.gz"),
        "ignored source material\n",
    )
    .unwrap();
    assert_eq!(run(root, &["new", "ed"]).status.code(), Some(1));
    fs::remove_file(repo.join("SPECS/ed/ignored.tar.gz")).unwrap();
    for (set, clear) in [
        ("--assume-unchanged", "--no-assume-unchanged"),
        ("--skip-worktree", "--no-skip-worktree"),
    ] {
        git(&repo, &["update-index", set, "SPECS/ed/ed.spec"]);
        fs::write(&path, "edit hidden from git status\n").unwrap();
        assert!(git(&repo, &["status", "--porcelain=v1"]).stdout.is_empty());
        let output = run(root, &["new", "ed"]);
        assert_eq!(output.status.code(), Some(1), "{set}: {output:?}");
        assert!(root.join("work/ed/.config.toml").is_file());
        assert!(!root.join("work/ed/checkout").exists());
        assert_file(&path, "edit hidden from git status\n");
        git(&repo, &["update-index", clear, "SPECS/ed/ed.spec"]);
        git(&repo, &["restore", "SPECS/ed/ed.spec"]);
    }
    assert_eq!(
        git(&repo, &["worktree", "list", "--porcelain"]).stdout,
        worktrees
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
    let checkout = area.join("checkout");
    let baseline = git(&checkout, &["rev-parse", "HEAD"]).stdout;
    let branch = git(&checkout, &["symbolic-ref", "HEAD"]).stdout;
    let config = fs::read(area.join(".config.toml")).unwrap();
    fs::write(checkout.join("README"), "local checkout edit\n").unwrap();
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
    drop(lock);
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
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]).stdout, baseline);
    assert_eq!(git(&checkout, &["symbolic-ref", "HEAD"]).stdout, branch);
    assert_file(checkout.join("README"), "local checkout edit\n");
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
fn successful_input_selection_preserves_pending_edits_and_previews_do_not_switch() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "review", "--pkgname", "ed"]));
    let area = root.join("work/review");
    let manifest = area.join("ed.toml");
    assert_eq!(binding(&area)["input"].as_str(), Some("authoring"));

    success(&run(
        root,
        &[
            "edit",
            "review",
            "--set",
            "package.version=1.22.6",
            "--prepare",
            "external-stage",
        ],
    ));
    assert_eq!(binding(&area)["input"].as_str(), Some("authoring"));
    assert!(!area.join("stage/.state/index.toml").exists());
    assert!(root.join("external-stage/ed.toml").is_file());

    success(&run(
        root,
        &["edit", "review", "--set", "package.version=1.22.7"],
    ));
    assert_eq!(binding(&area)["input"].as_str(), Some("edit"));
    let draft = area.join("stage/ed.toml");
    let index = area.join("stage/.state/index.toml");
    let pending = fs::read(&draft).unwrap();
    let identity = fs::read(&index).unwrap();
    assert_file(
        area.join("checkout/SPECS/ed/ed.spec"),
        include_str!("../fixtures/ed.spec"),
    );

    fs::write(&manifest, include_str!("../../examples/ed/ed.toml")).unwrap();
    success(&run(
        root,
        &["gen", "review", "--input", "authoring", "--offline"],
    ));
    assert_eq!(binding(&area)["input"].as_str(), Some("authoring"));
    for action in ["--check", "--diff"] {
        success(&run(root, &["edit", "review", action]));
        assert_eq!(binding(&area)["input"].as_str(), Some("authoring"));
        assert_eq!(fs::read(&draft).unwrap(), pending);
    }
    success(&run(
        root,
        &["edit", "review", "--set", "package.version=1.22.8"],
    ));
    assert_eq!(binding(&area)["input"].as_str(), Some("edit"));
    let updated_pending = fs::read(&draft).unwrap();
    let updated_identity = fs::read(&index).unwrap();
    assert_ne!(updated_pending, pending);
    assert_eq!(updated_identity, identity);

    let failed = run(root, &["new", "review"]);
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    for action in ["--stdout", "--diff", "--skip-existing"] {
        success(&run(root, &["new", "review", action]));
        assert_eq!(binding(&area)["input"].as_str(), Some("edit"));
        assert_eq!(fs::read(&draft).unwrap(), updated_pending);
        assert_eq!(fs::read(&index).unwrap(), updated_identity);
    }
    success(&run(root, &["new", "review", "--force"]));
    assert_eq!(binding(&area)["input"].as_str(), Some("authoring"));
    assert_eq!(fs::read(&draft).unwrap(), updated_pending);
    assert_eq!(fs::read(&index).unwrap(), updated_identity);
    fs::write(&manifest, include_str!("../../examples/ed/ed.toml")).unwrap();
    success(&run(root, &["gen", "review", "--offline"]));
    assert_file(
        area.join("stage/ed.candidate.spec"),
        include_str!("../fixtures/ed.spec"),
    );
    assert_eq!(fs::read(&draft).unwrap(), updated_pending);
}

#[test]
fn edit_copy_output_cannot_replace_work_inputs_or_binding_state() {
    let directory = workspace();
    let root = directory.path();
    success(&run(root, &["new", "review", "--pkgname", "ed"]));
    let area = root.join("work/review");
    let manifest = area.join("ed.toml");
    fs::write(&manifest, include_str!("../../examples/ed/ed.toml")).unwrap();
    success(&run(
        root,
        &["edit", "review", "--set=package.version=1.22.6"],
    ));
    success(&run(
        root,
        &["gen", "review", "--input=authoring", "--offline"],
    ));
    let target = area.join("checkout/SPECS/ed/ed.spec");
    let protected = [
        manifest,
        area.join("stage/ed.toml"),
        area.join(".config.toml"),
        area.join(".lock"),
    ];
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
        assert_eq!(binding(&area)["input"].as_str(), Some("authoring"));
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
    assert_eq!(
        run(uninitialized.path(), &["new", "demo", "--pkgname", "demo"])
            .status
            .code(),
        Some(1)
    );
    assert!(!uninitialized.path().join("work").exists());
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
fn author_uses_git_precedence_and_never_invents_a_missing_identity() {
    let directory = workspace();
    let root = directory.path();
    let repo = root.join("openruyi");
    for (key, value) in [
        ("user.name", "Local Author"),
        ("user.email", "local@example.org"),
    ] {
        assert!(
            Command::new("git")
                .args(["config", key, value])
                .current_dir(&repo)
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
    let from_config = || {
        command(root)
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env("GIT_CONFIG_GLOBAL", &global)
            .args(["new", "demo", "--pkgname", "demo", "--stdout"])
            .output()
            .unwrap()
    };
    let local = from_config();
    assert_eq!(
        document(&local)["spec"]["contributors"][0].as_str(),
        Some("Local Author <local@example.org>")
    );
    let environment = run(root, &["new", "demo", "--pkgname", "demo", "--stdout"]);
    assert_eq!(
        document(&environment)["spec"]["contributors"][0].as_str(),
        Some("Packager \"A\" \\测试 <packager@example.org>")
    );
    for key in ["user.name", "user.email"] {
        assert!(
            Command::new("git")
                .args(["config", "--unset", key])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    }
    let from_global = from_config();
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
        let output = cmd
            .args(["new", "demo", "--pkgname", "demo", "--stdout"])
            .output()
            .unwrap();
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
    // Keep the actual template's build configuration and add only ed's archive tool.
    scaffold["build-requires"]["rpm"]
        .as_array_mut()
        .unwrap()
        .push("lzip".into());
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
    assert_eq!(missing.status.code(), Some(1));
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
