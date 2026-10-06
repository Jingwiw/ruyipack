// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Edit stages are saved work, not implicit SPEC publication or static checks.

use super::super::{
    http::{Server, response},
    support::{machine_report, recipe_workspace},
};
use super::{SPEC, command, fixture, success};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Stdio};

fn stage(directory: &Path) -> std::path::PathBuf {
    directory.join(".ruyipack-draft/ed")
}

#[cfg(unix)]
fn editor(directory: &Path, text: &str) -> String {
    let source = directory.join("editor-input");
    fs::write(&source, text).unwrap();
    let script = directory.join("editor.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncp {} \"$1\"\n",
            shell_words::quote(&source.to_string_lossy())
        ),
    )
    .unwrap();
    format!("/bin/sh {}", shell_words::quote(&script.to_string_lossy()))
}

#[cfg(unix)]
#[test]
fn default_toml_editor_retains_unfinished_work_without_claiming_a_candidate() {
    let directory = fixture(SPEC);
    let incomplete = "[package]\nversion = \"unfinished\n";
    let editor = editor(directory.path(), incomplete);
    let result = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--field=package.version",
            "--editor",
            &editor,
        ])
        .output()
        .unwrap();
    success(&result);
    assert!(result.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC
    );
    assert_eq!(
        fs::read_to_string(stage(directory.path()).join("ed.toml")).unwrap(),
        incomplete
    );
    assert!(!stage(directory.path()).join("ed.candidate.spec").exists());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("static blockers"));
    // Reopening saved unfinished TOML must not parse it before the editor starts.
    for explicit in [false, true] {
        let mut request = command(directory.path());
        request.arg("--spec=ed.spec");
        if explicit {
            request.args(["--editor", "/usr/bin/true"]);
        } else {
            request.env("EDITOR", "/usr/bin/true");
        }
        let output = request.output().unwrap();
        success(&output);
        assert_eq!(
            fs::read_to_string(stage(directory.path()).join("ed.toml")).unwrap(),
            incomplete
        );
    }
    let expansion = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--field=package.summary",
            "--editor=/usr/bin/true",
        ])
        .output()
        .unwrap();
    assert_eq!(expansion.status.code(), Some(1), "{expansion:?}");
    assert!(String::from_utf8_lossy(&expansion.stderr).contains("repair saved TOML"));
    assert_eq!(
        fs::read_to_string(stage(directory.path()).join("ed.toml")).unwrap(),
        incomplete
    );
    let index: toml::Table = toml::from_str(
        &fs::read_to_string(stage(directory.path()).join(".state/index.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        index["drafts"][0]["fields"].as_array().unwrap(),
        &[toml::Value::String("package.version".into())]
    );
}

#[test]
fn cli_set_saves_toml_stage_and_does_not_run_static_checks() {
    let original = SPEC.replace("URL:            https://www.gnu.org/software/ed/\n", "");
    let directory = fixture(&original);
    let result = command(directory.path())
        .args(["--spec=ed.spec", "--set=package.version=2", "--format=toml"])
        .output()
        .unwrap();
    success(&result);
    let report = machine_report(&result);
    assert!(report.get("valid").is_none());
    assert_eq!(report["files"][0]["state"].as_str(), Some("pending-edit"));
    let values: toml::Value =
        toml::from_str(&fs::read_to_string(stage(directory.path()).join("ed.toml")).unwrap())
            .unwrap();
    assert_eq!(values["package"]["version"].as_str(), Some("2"));
    assert!(!stage(directory.path()).join("ed.candidate.spec").exists());
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        original
    );
}

#[cfg(unix)]
#[test]
fn saved_edits_preserve_binding_and_failed_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let work = recipe_workspace(directory.path(), "requested", "ed", SPEC);
    let config = work.join(".config.toml");
    let authoring = fs::read(&config).unwrap();
    for args in [
        vec!["--set=build-requires.rpm=[1]"],
        vec!["--field=package.version", "--editor=/usr/bin/false"],
    ] {
        let output = command(directory.path())
            .arg("requested")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(fs::read(&config).unwrap(), authoring);
        assert_eq!(
            fs::read_to_string(work.join("recipe/SPECS/ed/ed.spec")).unwrap(),
            SPEC
        );
    }
    let saved = command(directory.path())
        .args([
            "requested",
            "--set=package.license=not-a-valid-license",
            "--check",
            "--format=toml",
        ])
        .output()
        .unwrap();
    assert_eq!(saved.status.code(), Some(1), "{saved:?}");
    assert_eq!(
        machine_report(&saved)["files"][0]["state"].as_str(),
        Some("candidate")
    );
    assert_eq!(fs::read(&config).unwrap(), authoring);
    let pending: toml::Table =
        toml::from_str(&fs::read_to_string(work.join("ed.toml")).unwrap()).unwrap();
    assert_eq!(
        pending["package"]["license"].as_str(),
        Some("not-a-valid-license")
    );
    assert_eq!(
        fs::read_to_string(work.join("recipe/SPECS/ed/ed.spec")).unwrap(),
        SPEC
    );
}

#[test]
fn prepare_keeps_batches_external_and_preserves_managed_bindings() {
    let directory = tempfile::tempdir().unwrap();
    let first = recipe_workspace(directory.path(), "first", "ed", SPEC);
    let second = recipe_workspace(
        directory.path(),
        "second",
        "other",
        &SPEC.replace("Name:           ed", "Name:           other"),
    );
    let configs = [first.join(".config.toml"), second.join(".config.toml")];
    let before = configs
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();
    let managed = first.clone();
    let refused = command(directory.path())
        .args(["first", "second", "--field=package.version", "--prepare"])
        .arg(&managed)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("external DIR"));
    assert!(!managed.join(".state").exists());
    let external = directory.path().join("batch");
    success(
        &command(directory.path())
            .args(["first", "second", "--field=package.version", "--prepare"])
            .arg(&external)
            .output()
            .unwrap(),
    );
    let index: toml::Table =
        toml::from_str(&fs::read_to_string(external.join(".state/index.toml")).unwrap()).unwrap();
    assert_eq!(index["drafts"].as_array().unwrap().len(), 2);
    for (path, original) in configs.iter().zip(&before) {
        assert_eq!(&fs::read(path).unwrap(), original);
    }
    success(
        &command(directory.path())
            .args(["first", "--field=package.version", "--prepare"])
            .arg(&managed)
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(&configs[0]).unwrap(), before[0]);
    assert_eq!(fs::read(&configs[1]).unwrap(), before[1]);
    assert_eq!(
        fs::read_to_string(first.join("recipe/SPECS/ed/ed.spec")).unwrap(),
        SPEC
    );
    assert_eq!(
        fs::read_to_string(second.join("recipe/SPECS/other/other.spec")).unwrap(),
        SPEC.replace("Name:           ed", "Name:           other")
    );
}

#[test]
fn diff_caches_and_displays_the_same_candidate_without_implicit_check() {
    let directory = fixture(SPEC);
    let result = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set=package.url=ftp://example.org/new",
            "--diff",
        ])
        .output()
        .unwrap();
    success(&result);
    let diff = fs::read(stage(directory.path()).join("ed.diff")).unwrap();
    assert_eq!(result.stdout, diff);
    assert!(String::from_utf8_lossy(&diff).contains("+URL:            ftp://example.org/new"));
    let candidate = fs::read_to_string(stage(directory.path()).join("ed.candidate.spec")).unwrap();
    assert_eq!(
        candidate,
        SPEC.replace("https://www.gnu.org/software/ed/", "ftp://example.org/new")
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC
    );
    assert!(!String::from_utf8_lossy(&result.stderr).contains("static blockers"));
}

#[test]
fn check_diff_and_apply_are_orthogonal_but_failed_admission_blocks_apply() {
    let directory = fixture(SPEC);
    let result = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set=package.license=not-a-valid-license",
            "--check",
            "--diff",
            "--apply",
            "--format=toml",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let report = machine_report(&result);
    assert_eq!(report["success"].as_bool(), Some(false));
    assert_eq!(report["files"][0]["admissible"].as_bool(), Some(false));
    assert!(stage(directory.path()).join("ed.diff").is_file());
    assert!(stage(directory.path()).join("ed.candidate.spec").is_file());
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC
    );
}

#[test]
fn successful_apply_advances_stage_baseline_for_the_next_edit() {
    let directory = fixture(SPEC);
    let first = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set=package.version=2",
            "--check",
            "--diff",
            "--apply",
            "--format=toml",
        ])
        .output()
        .unwrap();
    success(&first);
    assert_eq!(
        machine_report(&first)["files"][0]["admissible"].as_bool(),
        Some(true)
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC.replace("Version:        1.22.5", "Version:        2")
    );
    let second = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set=package.summary=New summary",
            "--apply",
        ])
        .output()
        .unwrap();
    success(&second);
    let source = fs::read_to_string(directory.path().join("ed.spec")).unwrap();
    assert!(source.contains("Version:        2\n"));
    assert!(source.contains("Summary:        New summary\n"));
    let values: toml::Value =
        toml::from_str(&fs::read_to_string(stage(directory.path()).join("ed.toml")).unwrap())
            .unwrap();
    assert_eq!(values["package"]["version"].as_str(), Some("2"));
    assert_eq!(values["package"]["summary"].as_str(), Some("New summary"));
}

#[cfg(unix)]
#[test]
fn cli_editor_and_resume_preserve_one_toml_stage() {
    let directory = fixture(SPEC);
    success(
        &command(directory.path())
            .args([
                "--spec=ed.spec",
                "--field=package.version",
                "--editor=/usr/bin/true",
            ])
            .output()
            .unwrap(),
    );
    let draft = stage(directory.path()).join("ed.toml");
    success(
        &command(directory.path())
            .args(["--spec=ed.spec", "--set=package.version=2"])
            .output()
            .unwrap(),
    );
    let pending = fs::read(&draft).unwrap();
    success(
        &command(directory.path())
            .args([
                "--from",
                stage(directory.path()).to_str().unwrap(),
                "--editor=/usr/bin/true",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(&draft).unwrap(), pending);
    let checked = command(directory.path())
        .args([
            "--from",
            stage(directory.path()).to_str().unwrap(),
            "--check",
            "--format=toml",
        ])
        .output()
        .unwrap();
    success(&checked);
    assert_eq!(
        machine_report(&checked)["files"][0]["draft"].as_str(),
        Some(draft.canonicalize().unwrap().to_string_lossy().as_ref())
    );
    assert_eq!(fs::read(&draft).unwrap(), pending);
    let document: toml::Table = toml::from_str(std::str::from_utf8(&pending).unwrap()).unwrap();
    assert_eq!(document["package"]["version"].as_str(), Some("2"));
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC
    );
}

#[test]
fn hash_uses_edited_urls_and_includes_unmarked_signatures_but_not_local_files() {
    let server = Server::new(false, |path| response(path.as_bytes()));
    let original = format!(
        "Name: ed\nVersion: 1\nSource0: {}/%{{version}}.tar\nSource1: {}/%{{version}}.tar.sig\nSource2: local.txt\nSource3: {}/%{{version}}.tar\n%description\nDemo\n",
        server.url, server.url, server.url
    );
    let directory = fixture(&original);
    let result = server
        .command()
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .args([
            "edit",
            "--spec=ed.spec",
            "--set=package.version=2",
            "--hash",
            "--diff",
            "--format=toml",
        ])
        .output()
        .unwrap();
    success(&result);
    let record = &machine_report(&result)["files"][0];
    assert_eq!(
        record["source_hashes"]["sources"][0]["sha256"].as_str(),
        Some((format!("{:x}", Sha256::digest(b"/2.tar"))).as_str())
    );
    assert_eq!(
        record["source_hashes"]["sources"][1]["sha256"].as_str(),
        Some((format!("{:x}", Sha256::digest(b"/2.tar.sig"))).as_str())
    );
    assert!(
        record["source_hashes"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|source| source["number"].as_integer() != Some(2))
    );
    assert_eq!(
        record["source_hashes"]["sources"][2]["number"].as_integer(),
        Some(3)
    );
    assert_eq!(
        record["source_hashes"]["sources"][2]["sha256"],
        record["source_hashes"]["sources"][0]["sha256"]
    );
    assert_eq!(*server.calls.lock().unwrap(), ["/2.tar", "/2.tar.sig"]);
    let values: toml::Value =
        toml::from_str(&fs::read_to_string(stage(directory.path()).join("ed.toml")).unwrap())
            .unwrap();
    assert!(
        values["sources"]["0"]["sha256"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64)
    );
    let candidate = fs::read_to_string(stage(directory.path()).join("ed.candidate.spec")).unwrap();
    assert_eq!(candidate.matches("#!RemoteAsset:").count(), 3);
    assert!(candidate.contains("Source2: local.txt"));
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        original
    );
}

#[test]
fn pure_cli_arrays_and_read_only_operations_reuse_pending_stage_values() {
    let directory = fixture(SPEC);
    let result = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--set=package.version=2",
            "--set=build-requires.rpm=['autoconf', 'automake', 'libtool', 'make', 'xz',]",
        ])
        .output()
        .unwrap();
    success(&result);
    let draft = stage(directory.path()).join("ed.toml");
    let pending = fs::read(&draft).unwrap();
    for array in ["[1]", "['make', false]", "['make'] trailing"] {
        let output = command(directory.path())
            .args(["--spec=ed.spec", "--format=toml"])
            .arg(format!("--set=build-requires.rpm={array}"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(
            machine_report(&output)["error"]["code"].as_str(),
            Some("invalid-assignment")
        );
        assert_eq!(fs::read(&draft).unwrap(), pending);
    }
    let preview = command(directory.path())
        .args(["--spec=ed.spec", "--diff"])
        .output()
        .unwrap();
    success(&preview);
    let candidate = fs::read_to_string(stage(directory.path()).join("ed.candidate.spec")).unwrap();
    assert!(candidate.contains("Version:        2\n"));
    assert!(candidate.contains("BuildRequires:  xz\n"));
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        SPEC
    );
    let apply = command(directory.path())
        .args(["--spec=ed.spec", "--apply"])
        .output()
        .unwrap();
    success(&apply);
    assert_eq!(
        fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
        candidate
    );
    assert!(stage(directory.path()).join("ed.toml").is_file());
}

#[test]
fn failed_rebase_index_retains_loadable_original_and_reports_published_spec() {
    let directory = fixture(SPEC);
    let prepared = command(directory.path())
        .args([
            "--spec=ed.spec",
            "--field=package.version",
            "--prepare=drafts",
        ])
        .output()
        .unwrap();
    success(&prepared);
    let draft_root = directory.path().join("drafts");
    let draft = draft_root.join("ed.toml");
    super::change_version(&draft, "2");
    let index = draft_root.join(".state/index.toml");
    let before_index = fs::read(&index).unwrap();
    let index_value: toml::Value =
        toml::from_str(std::str::from_utf8(&before_index).unwrap()).unwrap();
    let digest = index_value["drafts"][0]["original_sha256"]
        .as_str()
        .unwrap();
    let original = draft_root.join(format!(".state/originals/0-{digest}.spec"));
    assert_eq!(fs::read_to_string(&original).unwrap(), SPEC);
    fs::hard_link(&index, directory.path().join("index-link")).unwrap();

    let applied = command(directory.path())
        .args(["--from=drafts", "--apply", "--format=toml"])
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(1), "{applied:?}");
    assert!(applied.stderr.is_empty());
    let receipt = machine_report(&applied);
    let source = directory.path().join("ed.spec").canonicalize().unwrap();
    assert_eq!(
        receipt["written"],
        toml::Value::Array(vec![source.to_str().unwrap().into()])
    );
    assert_eq!(receipt["success"].as_bool(), Some(false));
    assert_eq!(receipt["valid"].as_bool(), Some(true));
    assert!(
        receipt["error"]["message"]
            .as_str()
            .unwrap()
            .contains("multiple hard links")
    );
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        super::version_source("2")
    );
    assert_eq!(fs::read(&index).unwrap(), before_index);
    assert_eq!(fs::read_to_string(&original).unwrap(), SPEC);

    // Loading succeeds; the changed source is correctly stale relative to the
    // previous indexed baseline, not an unrecoverable baseline hash mismatch.
    let check = command(directory.path())
        .args(["--from=drafts", "--check", "--format=toml"])
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(1), "{check:?}");
    let receipt = machine_report(&check);
    assert_eq!(
        receipt["files"][0]["error"]["code"].as_str(),
        Some("source-changed")
    );
    assert!(!String::from_utf8_lossy(&check.stdout).contains("saved original hash mismatch"));
}

#[cfg(unix)]
#[test]
fn candidate_actions_never_launch_an_editor_based_on_saved_state() {
    for saved in [false, true] {
        for action in ["--diff", "--apply", "--check"] {
            let directory = fixture(SPEC);
            let marker = directory.path().join("editor-called");
            let script = directory.path().join("editor-marker.sh");
            fs::write(
                &script,
                format!(
                    "#!/bin/sh\ntouch {}\n",
                    shell_words::quote(&marker.to_string_lossy())
                ),
            )
            .unwrap();
            if saved {
                success(
                    &command(directory.path())
                        .args(["--spec=ed.spec", "--set=package.version=2"])
                        .output()
                        .unwrap(),
                );
            }
            let result = command(directory.path())
                .args(["--spec=ed.spec", action])
                .env(
                    "GIT_EDITOR",
                    format!("/bin/sh {}", shell_words::quote(&script.to_string_lossy())),
                )
                .output()
                .unwrap();
            assert!(!marker.exists(), "{saved} {action}: {result:?}");
            if saved || action == "--check" {
                success(&result);
            } else {
                assert_eq!(result.status.code(), Some(1));
                assert!(String::from_utf8_lossy(&result.stderr).contains("no saved edits"));
                assert!(!stage(directory.path()).exists());
            }
            let expected = if saved && action == "--apply" {
                SPEC.replace("Version:        1.22.5", "Version:        2")
            } else {
                SPEC.to_owned()
            };
            assert_eq!(
                fs::read_to_string(directory.path().join("ed.spec")).unwrap(),
                expected
            );
        }
    }
}

#[test]
fn script_assignment_replaces_and_addition_appends_checked_text() {
    let directory = fixture(SPEC);
    let run = |args: &[&str]| {
        command(directory.path())
            .args(["--spec=ed.spec", "--apply"])
            .args(args)
            .output()
            .unwrap()
    };
    success(&run(&["--add", "build.stages.conf.prepend=echo first"]));
    success(&run(&[
        "--add",
        "build.stages.conf.prepend=echo second",
        "--add",
        "build.stages.conf.prepend=echo third",
    ]));
    let source = fs::read_to_string(directory.path().join("ed.spec")).unwrap();
    assert!(source.contains("%conf -p\necho first\necho second\necho third\n"));
    success(&run(&[
        "--set",
        "build.stages.conf.prepend=echo replacement",
    ]));
    let expected = fs::read_to_string(directory.path().join("ed.spec")).unwrap();
    assert_eq!(
        expected,
        source.replace("echo first\necho second\necho third", "echo replacement")
    );
    for args in [
        vec![
            "--set",
            "build.stages.conf.prepend=echo one",
            "--add",
            "build.stages.conf.prepend=echo two",
        ],
        vec!["--add", "build.stages.conf.prepend=%files\n/unselected"],
        vec!["--add", "package.version=2"],
    ] {
        let rejected = fixture(&expected);
        let result = command(rejected.path())
            .args(["--spec=ed.spec", "--apply"])
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert_eq!(
            fs::read_to_string(rejected.path().join("ed.spec")).unwrap(),
            expected
        );
    }
}
