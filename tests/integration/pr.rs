// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

use super::support::{git, isolated_command, machine_report, recipe_workspace, success};
use std::{fs, path::Path, process::Output};

fn run(root: &Path) -> Output {
    isolated_command(env!("CARGO_BIN_EXE_ruyipack"), root)
        .args(["pr", "--plan", "plan.toml", "--format=toml"])
        .output()
        .unwrap()
}

#[test]
fn pr_preview_binds_plan_to_commits_and_does_not_publish() {
    let root = tempfile::tempdir().unwrap();
    let spec = include_str!("../fixtures/ed.spec");
    let area = recipe_workspace(root.path(), "review", "ed", spec);
    success(
        &isolated_command(env!("CARGO_BIN_EXE_ruyipack"), root.path())
            .args(["edit", "review", "--set=package.version=1.22.6", "--apply"])
            .output()
            .unwrap(),
    );
    let repo = root.path().join("openruyi");
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:alice/packages.git",
        ],
    );
    git(&repo, &["checkout", "-qb", "fix/ed"]);
    let changed = spec.replace("1.22.5", "1.22.6");
    fs::write(area.join("recipe/SPECS/ed/ed.spec"), &changed).unwrap();
    fs::write(repo.join("SPECS/ed/ed.spec"), &changed).unwrap();
    git(
        &repo,
        &[
            "commit",
            "-qam",
            "SPECS: ed: Update to 1.22.6\n\nAction: Update to 1.22.6",
        ],
    );
    fs::write(
        root.path().join("body.md"),
        "## Summary\n{{summary}}\n{{obs_links}}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("plan.toml"),
        "[[packages]]\nwork='review'\n[pr]\ntitle='Update ed'\nbase='main'\ntemplate='body.md'\n",
    )
    .unwrap();
    fs::write(
        area.join("recipe/SPECS/ed/_constraints"),
        "<constraints/>\n",
    )
    .unwrap();
    let before = git(&repo, &["rev-parse", "HEAD"]).stdout;
    let output = run(root.path());
    success(&output);
    let report = machine_report(&output);
    assert_eq!(report["target"].as_str(), Some("alice/packages"));
    assert_eq!(
        report["body"].as_str(),
        Some("## Summary\n- ed: Update to 1.22.6\n\n")
    );
    assert_eq!(report["pushed"].as_bool(), Some(false));
    assert_eq!(report["works"].as_array().unwrap().len(), 1);
    assert_eq!(report["commits"].as_array().unwrap().len(), 1);
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]).stdout, before);
    for (note, expected) in [
        ("ed: Tests need a terminal.", 0),
        ("other: Tests need a terminal.", 1),
        ("ed: first\nsecond", 2),
        ("ed:", 2),
    ] {
        let output = isolated_command(env!("CARGO_BIN_EXE_ruyipack"), root.path())
            .args(["pr", "--plan=plan.toml", "--format=toml", "--note", note])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected));
        if expected == 0 {
            assert!(
                machine_report(&output)["body"]
                    .as_str()
                    .unwrap()
                    .contains("  - Tests need a terminal.")
            );
        }
    }
    fs::write(area.join("recipe/SPECS/ed/ed.spec"), spec).unwrap();
    let failed = run(root.path());
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        machine_report(&failed)["error"]
            .as_str()
            .unwrap()
            .contains("WORK differs")
    );
    fs::write(area.join("recipe/SPECS/ed/ed.spec"), changed).unwrap();
    fs::write(repo.join("unrelated"), "not selected").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "Unrelated change"]);
    let failed = run(root.path());
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        machine_report(&failed)["error"]
            .as_str()
            .unwrap()
            .contains("outside plan")
    );
}
