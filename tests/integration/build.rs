// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Host lifecycle contract tests use a fake Docker executable, never a daemon.

#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::Output,
};

#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;

use serde_json::Value;
use tempfile::TempDir;

use super::support;
use super::support::git;

const DOCKER: &str = r"#!/usr/bin/env python3
import hashlib, json, os, pathlib, shutil, sys, time
args = sys.argv[1:]
state = pathlib.Path(os.environ['RPK_FAKE_STATE'])
mode = os.environ.get('RPK_FAKE_MODE', '')
with (state / 'calls.jsonl').open('a') as stream:
    stream.write(json.dumps(args) + '\n')
if mode.startswith('default-context'):
    assert args[0] != '--context', args
else:
    assert args[:2] == ['--context', 'personalDocker'], args
    args = args[2:]
if mode == 'timeout-descendant' and args[0] == 'context':
    pid = os.fork()
    if pid == 0:
        time.sleep(12)
        (state / 'descendant-survived').write_text('unexpected survivor')
        os._exit(0)
    (state / 'descendant.pid').write_text(str(pid))
    time.sleep(60)
if args[0] == 'context': print(json.dumps([{'Name': 'personalDocker', 'Endpoints': {'docker': {'Host': 'fixture://daemon'}}}]))
elif args[:2] == ['compose', 'version']: print('fixture-compose')
elif args[0] == 'compose':
    operation = args[args.index('--project-name') + 2]
    if operation == 'config': print(json.dumps({'services': {'worker': {'platform': 'linux/amd64'}}}))
    if operation == 'create': (state / 'project').write_text(args[args.index('--project-name') + 1])
    if operation == 'ps': print('abcdef0123456789')
    if operation == 'down' and mode in ('cleanup-failure', 'engine-cleanup-failure', 'bad-hash-cleanup'): sys.exit(19)
elif args[0] == 'info': print(json.dumps({'ID': 'fixture-daemon', 'OSType': 'windows' if mode == 'windows-daemon' else 'linux', 'ServerVersion': 'fixture', 'Architecture': 'x86_64', 'KernelVersion': 'fixture-kernel'}))
elif args[:2] == ['image', 'ls']:
    if mode == 'owned-images': print('\n'.join('sha256:' + x * 64 for x in '123'))
elif args[:2] == ['image', 'rm']:
    assert mode == 'owned-images' and args[2] == 'sha256:' + '1' * 64, args
elif args[:2] == ['image', 'inspect']:
    if mode == 'owned-images' and args[2] != 'sha256:fixture-image':
        project = (state / 'project').read_text()
        tags = ['ruyipack/mock-openruyi:local'] if args[2].endswith('2') else [project + '-worker:latest']
        print(json.dumps([{'Config': {'Labels': {'com.docker.compose.project': project}}, 'RepoTags': tags}]))
    else: print(json.dumps([{'Architecture': 'amd64'}]))
elif args[0] == 'inspect': print(json.dumps([{'Image': 'sha256:fixture-image', 'Config': {'Env': [], 'Labels': {'com.docker.compose.project': 'other' if mode == 'wrong-label' else (state / 'project').read_text()}}, 'State': {'Running': mode == 'running-worker'}, 'HostConfig': {'Privileged': True}, 'Mounts': []}]))
elif args[0] == 'cp':
    if args[1].startswith('abcdef0123456789:'):
        if mode in ('copy-failure', 'copy-failure-retain'): sys.exit(17)
        source = state / 'engine'
        if source.exists(): shutil.copytree(source, args[2], dirs_exist_ok=True)
    else:
        shutil.copytree(args[1], state / 'input', dirs_exist_ok=True)
elif args[0] == 'exec':
    if '-c' in args and 'Probe this worker only' in args[args.index('-c')+1]:
        if mode == 'probe-failure': print('mount denied'); sys.exit(31)
        print(json.dumps({'target_arch':'x86_64', 'rpm_arch':'x86_64', 'image_arch':'amd64', 'mount':True, 'chroot':True, 'translator':'unknown'})); sys.exit(0)
    if '-c' in args and args[1] != '--interactive':
        for name in ('input', 'engine'):
            shutil.rmtree(state / name, ignore_errors=True)
        sys.exit(0)
    if args[1] == '--interactive':
        assert args[1:3] == ['--interactive', 'abcdef0123456789'], args
        if '--' in args: print(json.dumps(args[args.index('--')+1:]))
        print('mock chroot shell fixture')
        sys.exit(42 if mode == 'shell-failure' else 0)
    assert args[1] == 'abcdef0123456789'
    (state / 'engine').mkdir(exist_ok=True)
    (state / 'engine' / 'artifact.rpm').write_bytes(b'fixture RPM bytes')
    (state / 'engine' / 'mock.cfg').write_text('fixture')
    target = {'architecture': 'x86_64', 'release_policy': 'fixture'}
    (state / 'engine' / 'target.json').write_text(json.dumps(target))
    receipt = {'target': target, 'mock_config_sha256': hashlib.sha256(b'fixture').hexdigest(),
               'installed_packages': [{'name':'rpm','evr':'6','arch':'x86_64'}], 'collection_errors': [],
               'format_version': 1, 'engine': 'mock', 'target_stage': args[args.index('--stage')+1] if '--stage' in args else 'build', 'success': mode not in ('failed-receipt', 'default-context-failure'),
               'artifacts': [{'path': 'artifact.rpm', 'size': len(b'fixture RPM bytes'),
                 'sha256': '0' * 64 if mode in ('bad-hash', 'bad-hash-cleanup') else hashlib.sha256(b'fixture RPM bytes').hexdigest(),
                 'identity': 'fixture\t1\t1.or\tx86_64'}]}
    if mode != 'no-receipt':
        (state / 'engine' / 'receipt.json').write_text(json.dumps(receipt))
    print('engine stdout', flush=True)
    print('engine stderr', file=sys.stderr, flush=True)
    if mode.startswith('interrupt'):
        (state / 'engine-ready').write_text(str(os.getpid()))
        time.sleep(60)
    if mode == 'timeout': time.sleep(60)
    if mode in ('engine-failure', 'engine-cleanup-failure'): sys.exit(23)
elif args[0] in ('container', 'network', 'volume'):
    kind, operation = args[:2]
    if operation == 'ls':
        if mode == 'owned-images' and any(x.startswith('ancestor=') and x.endswith('3') for x in args): print('1111111111111111')
        else: print({'container':'abcdef0123456789','network':'fedcba9876543210','volume':'owned-volume\nshared-volume'}[kind])
    elif operation == 'inspect':
        labels = {'com.docker.compose.project': 'different-project' if mode == 'wrong-label' else (state / 'project').read_text()}
        if kind == 'volume' and args[2] == 'owned-volume': labels['com.docker.compose.volume'] = 'data'
        print(json.dumps([{'Config': {'Labels': labels}, 'Labels': labels}]))
    elif operation == 'rm' and kind == 'network' and mode == 'remove-network-failure': sys.exit(29)
    elif operation != 'rm': raise AssertionError(args)
elif args[0] in ('start', 'kill', 'stop'): pass
else: raise AssertionError(args)
";

struct Fixture {
    root: TempDir,
    bin: PathBuf,
    spec: PathBuf,
    source: PathBuf,
    environment: PathBuf,
    output: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let source = root.path().join("sources");
        let environment = root.path().join("environment");
        for directory in [&bin, &source, &environment] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(bin.join("docker"), DOCKER).unwrap();
        fs::set_permissions(bin.join("docker"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(source.join("source.txt"), "original material\n").unwrap();
        fs::write(
            environment.join("compose.yaml"),
            "services:\n  worker:\n    image: fixture\n",
        )
        .unwrap();
        let spec = root.path().join("fixture.spec");
        fs::write(&spec, "Name: fixture\nVersion: 1\n").unwrap();
        let output = root.path().join("build/fixture");
        Self {
            root,
            bin,
            spec,
            source,
            environment,
            output,
        }
    }

    fn command(&self, mode: &str) -> std::process::Command {
        let path = std::env::join_paths(std::iter::once(self.bin.clone()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let mut command = support::command();
        command
            .env("PATH", path)
            .env("RPK_FAKE_STATE", self.root.path())
            .env("RPK_FAKE_MODE", mode);
        command
    }

    fn build_command(&self, mode: &str, timeout: u64) -> std::process::Command {
        let mut command = self.command(mode);
        command.arg("build").arg("--spec").arg(&self.spec);
        if mode != "default-context" {
            command.args(["--context", "personalDocker"]);
        }
        if !matches!(
            mode,
            "default-retain" | "copy-failure-retain" | "interrupt-retain"
        ) {
            command.arg("--rm");
        }
        if mode != "builtin" {
            command
                .arg("--config")
                .arg(self.environment.join("compose.yaml"));
        }
        command
            .arg("--source-dir")
            .arg(&self.source)
            .arg("--output-dir")
            .arg(self.output.parent().unwrap())
            .arg("--timeout")
            .arg(timeout.to_string())
            .args(["--format", "toml"]);
        command
    }

    fn run(&self, mode: &str, timeout: u64) -> Output {
        self.build_command(mode, timeout).output().unwrap()
    }

    fn receipt(&self) -> Value {
        serde_json::from_slice(&fs::read(self.output.join("receipt.json")).unwrap()).unwrap()
    }

    fn reported_receipt(&self, output: &Output) -> Value {
        let outcome = support::machine_report(output);
        assert_eq!(outcome["operation"].as_str(), Some("build"));
        assert_eq!(outcome["success"].as_bool(), Some(true));
        let path = self.output.join("receipt.json").canonicalize().unwrap();
        assert_eq!(outcome["receipt"].as_str().unwrap(), path.to_string_lossy());
        let receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(receipt, self.receipt());
        receipt
    }

    fn workspace(&self, specs: &str, package: &str, spec_name: &str) -> PathBuf {
        let workspace = self.root.path().join("workspace");
        support::success(
            &self
                .command("default-context")
                .arg("init")
                .arg(&workspace)
                .output()
                .unwrap(),
        );
        fs::write(
            workspace.join(".ruyiconfig/config.toml"),
            format!("recipes = 'recipes'\nwork = 'areas'\nspecs = {specs:?}\n"),
        )
        .unwrap();
        let recipes = workspace.join("recipes");
        let package_directory = recipes.join(specs).join(package);
        fs::create_dir_all(&package_directory).unwrap();
        fs::write(
            package_directory.join(spec_name),
            "Name: fixture\nVersion: 1\n",
        )
        .unwrap();
        fs::write(
            package_directory.join("archive.txt"),
            "named recipe material\n",
        )
        .unwrap();
        git(&recipes, &["init", "--quiet", "--initial-branch=main"]);
        git(&recipes, &["add", "."]);
        git(
            &recipes,
            &["commit", "--quiet", "-m", "Named build fixture"],
        );
        workspace
    }
}

#[test]
fn named_build_and_shell_share_binding_recipe_and_workspace_configuration() {
    let mut fixture = Fixture::new();
    let workspace = fixture.workspace("distro/recipes", "fixture", "fixture.spec");
    let recipes = workspace.join("recipes");
    fs::write(
        recipes.join("distro/recipes/fixture/other.spec"),
        "Name: other\n",
    )
    .unwrap();
    git(&recipes, &["add", "."]);
    git(
        &recipes,
        &["commit", "--quiet", "-m", "Additional package material"],
    );
    let cwd = workspace.join("nested");
    fs::create_dir(&cwd).unwrap();
    fs::write(cwd.join("fixture-test"), "cwd decoy must not be read\n").unwrap();
    let assets = workspace.join(".ruyiconfig/build");
    let custom_compose = "services:\n  worker:\n    image: fixture-workspace\n";
    fs::write(assets.join("compose.yaml"), custom_compose).unwrap();
    fs::write(assets.join("Dockerfile"), "FROM workspace-fixture\n").unwrap();
    for name in ["compose.yaml", "Dockerfile"] {
        fs::set_permissions(assets.join(name), fs::Permissions::from_mode(0o600)).unwrap();
    }
    fixture.output = workspace.join("areas/fixture-test/build");
    let output = fixture
        .command("default-context")
        .current_dir(&cwd)
        .args([
            "build",
            "fixture-test",
            "--pkgname",
            "fixture",
            "--format",
            "toml",
        ])
        .output()
        .unwrap();
    support::success(&output);
    let receipt = fixture.reported_receipt(&output);
    assert_eq!(receipt["package"], "fixture");
    assert!(receipt["context"].is_null());
    let area = workspace.join("areas/fixture-test");
    let binding: toml::Table =
        toml::from_str(&fs::read_to_string(area.join(".config.toml")).unwrap()).unwrap();
    assert_eq!(binding["pkg"].as_str(), Some("fixture"));
    let package_directory = area.join("recipe/SPECS/fixture");
    assert_eq!(
        receipt["spec"],
        fs::canonicalize(package_directory.join("fixture.spec"))
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        receipt["source_dir"],
        fs::canonicalize(&package_directory)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        fs::read(fixture.output.join("input/SOURCES/archive.txt")).unwrap(),
        b"named recipe material\n"
    );
    assert!(fixture.output.join("input/SPECS/fixture.spec").is_file());
    assert_eq!(
        fs::read_to_string(fixture.output.join(".config/compose.yaml")).unwrap(),
        custom_compose
    );
    assert_eq!(
        fs::read_to_string(fixture.output.join(".config/Dockerfile")).unwrap(),
        "FROM workspace-fixture\n"
    );
    assert_eq!(receipt["configuration_files"].as_array().unwrap().len(), 4);
    for name in ["compose.yaml", "Dockerfile"] {
        assert_eq!(
            fs::metadata(assets.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(
        fs::read_to_string(assets.join("compose.yaml")).unwrap(),
        custom_compose
    );
    assert!(!cwd.join("build").exists());
    assert!(!workspace.join("build").exists());
    assert_eq!(
        fs::read_to_string(cwd.join("fixture-test")).unwrap(),
        "cwd decoy must not be read\n"
    );
    let receipt_bytes = fs::read(fixture.output.join("receipt.json")).unwrap();
    let binding_lock = fs::File::open(area.join(".lock")).unwrap();
    binding_lock.try_lock().unwrap();
    let calls_before = fs::read(fixture.root.path().join("calls.jsonl")).unwrap();
    for operation in ["build", "shell", "clean"] {
        let blocked = fixture
            .command("default-context")
            .current_dir(&cwd)
            .args([operation, "fixture-test"])
            .output()
            .unwrap();
        assert_eq!(blocked.status.code(), Some(1), "{blocked:?}");
        assert!(support::output_text(&blocked.stderr).contains("cannot lock development area"));
    }
    assert_eq!(
        fs::read(fixture.root.path().join("calls.jsonl")).unwrap(),
        calls_before
    );
    binding_lock.unlock().unwrap();
    let unreachable_recipes = workspace.join("temporarily-unreachable-recipes");
    fs::rename(&recipes, &unreachable_recipes).unwrap();
    let shell = fixture
        .command("default-context")
        .current_dir(package_directory.parent().unwrap())
        .args(["shell", "fixture-test"])
        .output()
        .unwrap();
    support::success(&shell);
    assert!(support::output_text(&shell.stdout).contains("mock chroot shell fixture"));
    assert_eq!(
        fs::read(fixture.output.join("receipt.json")).unwrap(),
        receipt_bytes
    );
    fs::rename(&unreachable_recipes, &recipes).unwrap();
    let repeated = fixture
        .command("default-context")
        .current_dir(&cwd)
        .args(["build", "fixture-test", "--format", "toml"])
        .output()
        .unwrap();
    support::success(&repeated);
    let history = fixture.output.parent().unwrap().join("build-history");
    let previous = fs::read_dir(&history)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::read(previous.join("receipt.json")).unwrap(),
        receipt_bytes
    );
    // The fake daemon models one inspect identity at a time; select the archived worker.
    let current_project = fs::read(fixture.root.path().join("project")).unwrap();
    fs::write(
        fixture.root.path().join("project"),
        receipt["execution"]["details"]["project"].as_str().unwrap(),
    )
    .unwrap();
    support::success(
        &fixture
            .command("default-context")
            .current_dir(&cwd)
            .args(["shell", "fixture-test", "--attempt"])
            .arg(previous.file_name().unwrap())
            .output()
            .unwrap(),
    );
    fs::write(fixture.root.path().join("project"), current_project).unwrap();
    let manifest = area.join("fixture.toml");
    fs::write(&manifest, "authoring input, not a build artifact\n").unwrap();
    let head = fs::read(area.join(".config.toml")).unwrap();
    fs::rename(&recipes, &unreachable_recipes).unwrap();
    support::success(
        &fixture
            .command("default-context")
            .current_dir(&cwd)
            .args([
                "clean",
                "fixture-test",
                "--attempt",
                "current",
                "--force",
                "--format",
                "toml",
            ])
            .output()
            .unwrap(),
    );
    assert!(!fixture.output.exists());
    assert_eq!(
        fs::read_to_string(&manifest).unwrap(),
        "authoring input, not a build artifact\n"
    );
    assert_eq!(fs::read(area.join(".config.toml")).unwrap(), head);
    assert!(package_directory.join("fixture.spec").is_file());
}

#[test]
fn named_build_refuses_directory_overrides_and_missing_specs_before_creation() {
    let fixture = Fixture::new();
    let workspace = fixture.workspace("SPECS", "fixture", "fixture.spec");
    for flag in ["--output-dir", "--source-dir"] {
        let output = fixture
            .command("default-context")
            .current_dir(&workspace)
            .args(["build", "named", "--pkgname", "fixture", flag])
            .arg(&fixture.source)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(support::output_text(&output.stderr).contains(flag));
        assert!(!workspace.join("areas").exists());
    }
    let absent = fixture
        .command("default-context")
        .current_dir(&workspace)
        .args(["build", "absent"])
        .output()
        .unwrap();
    assert_eq!(absent.status.code(), Some(1), "{absent:?}");
    assert!(!workspace.join("areas").exists());
    assert!(!fixture.root.path().join("calls.jsonl").exists());
    let missing_shell = fixture
        .command("default-context")
        .current_dir(&workspace)
        .args(["shell", "named"])
        .output()
        .unwrap();
    assert_eq!(missing_shell.status.code(), Some(1), "{missing_shell:?}");
    assert!(!workspace.join("areas").exists());
}

#[test]
fn build_copies_inputs_and_returns_a_receipt_without_context_mutation() {
    let fixture = Fixture::new();
    let output = fixture.run("", 10);
    support::success(&output);
    assert!(support::output_text(&output.stderr).contains("engine stdout"));
    assert!(support::output_text(&output.stderr).contains("engine stderr"));
    let receipt = fixture.reported_receipt(&output);
    assert_eq!(receipt["execution"]["success"], true);
    assert_eq!(receipt["success"], true);
    assert!(receipt["engine_validation_error"].is_null());
    assert_eq!(
        receipt["execution"]["details"]["image_id"],
        "sha256:fixture-image"
    );
    assert_eq!(receipt["inputs"].as_array().unwrap().len(), 3);
    assert_eq!(
        fs::read(fixture.output.join("engine/artifact.rpm")).unwrap(),
        b"fixture RPM bytes"
    );
    assert_eq!(
        fs::read(fixture.root.path().join("input/SPECS/fixture.spec")).unwrap(),
        fs::read(&fixture.spec).unwrap()
    );
    assert_eq!(
        fs::read(fixture.root.path().join("input/SOURCES/source.txt")).unwrap(),
        b"original material\n"
    );
    for command in receipt["execution"]["commands"].as_array().unwrap() {
        assert_eq!(command["argv"][0], "docker");
        assert_eq!(command["argv"][1], "--context");
        assert_eq!(command["argv"][2], "personalDocker");
        assert_eq!(command["exit_code"], 0);
        assert_eq!(command["started"], true);
        assert!(
            !command["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == "use")
        );
        assert!(
            fixture
                .output
                .join(command["stdout"].as_str().unwrap())
                .is_file()
        );
        assert!(
            fixture
                .output
                .join(command["stderr"].as_str().unwrap())
                .is_file()
        );
        if command["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "/input/engine.py")
        {
            let terminal = String::from_utf8_lossy(&output.stderr);
            assert!(terminal.contains("engine stdout\n"), "{terminal}");
            assert!(terminal.contains("engine stderr\n"), "{terminal}");
            assert_eq!(
                fs::read(fixture.output.join(command["stdout"].as_str().unwrap())).unwrap(),
                b"engine stdout\n"
            );
            assert_eq!(
                fs::read(fixture.output.join(command["stderr"].as_str().unwrap())).unwrap(),
                b"engine stderr\n"
            );
        }
    }
    assert_eq!(
        fs::read(&fixture.spec).unwrap(),
        b"Name: fixture\nVersion: 1\n"
    );
    assert_eq!(
        fs::read(fixture.source.join("source.txt")).unwrap(),
        b"original material\n"
    );
}

#[test]
fn build_retains_a_stopped_worker_by_default() {
    let fixture = Fixture::new();
    let output = fixture.run("default-retain", 30);
    support::success(&output);
    let report = support::machine_report(&output);
    assert_eq!(report["next_steps"]["shell"][1].as_str(), Some("shell"));
    assert!(std::path::Path::new(report["artifacts"].as_str().unwrap()).is_dir());
    let receipt = fixture.receipt();
    assert_eq!(receipt["remove_requested"], false);
    let execution = &receipt["execution"];
    assert_eq!(execution["cleanup_skipped"], true);
    let calls = execution["commands"].as_array().unwrap();
    assert!(calls.iter().any(|c| c["argv"][3] == "stop"));
    assert!(
        !calls
            .iter()
            .any(|c| c["argv"].as_array().unwrap().iter().any(|a| a == "down"))
    );
    assert_eq!(execution["recovery_commands"][0][3], "start");
    assert!(fixture.output.join("engine/artifact.rpm").is_file());
}

#[test]
fn build_preserves_primary_engine_failure_and_cleanup_failure() {
    for (mode, field, code) in [
        ("engine-failure", "failure", 23),
        ("cleanup-failure", "cleanup_failure", 19),
    ] {
        let fixture = Fixture::new();
        let output = fixture.run(mode, 10);
        assert_eq!(output.status.code(), Some(1));
        let receipt = fixture.receipt();
        assert_eq!(receipt["execution"]["success"], false);
        assert!(
            receipt["execution"][field]
                .as_str()
                .unwrap()
                .contains(&code.to_string())
        );
        assert!(fixture.output.join("engine/artifact.rpm").is_file());
        if mode == "engine-failure" {
            let commands = receipt["execution"]["commands"].as_array().unwrap();
            let stop = commands
                .iter()
                .position(|c| c["argv"][3] == "stop")
                .unwrap();
            let copy = commands.iter().rposition(|c| c["argv"][3] == "cp").unwrap();
            assert!(stop < copy);
        }
    }
}

#[test]
fn cleanup_error_does_not_replace_the_primary_engine_error() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.run("engine-cleanup-failure", 10).status.code(),
        Some(1)
    );
    let receipt = fixture.receipt();
    assert!(
        receipt["execution"]["failure"]
            .as_str()
            .unwrap()
            .contains("23")
    );
    assert!(
        receipt["execution"]["cleanup_failure"]
            .as_str()
            .unwrap()
            .contains("19")
    );
}

#[test]
fn artifact_copy_failure_retains_worker_and_recovery_commands() {
    let fixture = Fixture::new();
    let output = fixture.run("copy-failure-retain", 30);
    assert_eq!(output.status.code(), Some(1));
    let receipt = fixture.receipt();
    let execution = &receipt["execution"];
    assert_eq!(execution["success"], false);
    assert_eq!(execution["cleanup_skipped"], true);
    assert!(
        execution["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["argv"][3] == "stop")
    );
    assert!(execution["artifact_error"].as_str().unwrap().contains("17"));
    assert_eq!(execution["recovery_commands"].as_array().unwrap().len(), 3);
    assert!(
        !execution["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == "down"))
    );
}

#[test]
fn explicit_removal_is_attempted_even_when_evidence_retrieval_fails() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run("copy-failure", 30).status.code(), Some(1));
    let receipt = fixture.receipt();
    assert_eq!(receipt["remove_requested"], true);
    assert_eq!(receipt["execution"]["cleanup_skipped"], false);
    assert!(
        receipt["execution"]["artifact_error"]
            .as_str()
            .unwrap()
            .contains("17")
    );
    assert!(
        receipt["execution"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["argv"].as_array().unwrap().iter().any(|a| a == "down"))
    );
    assert!(
        receipt["execution"]["recovery_commands"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn timeout_stops_worker_then_retrieves_partial_evidence_before_cleanup() {
    let fixture = Fixture::new();
    let output = fixture.run("timeout", 10);
    assert_eq!(output.status.code(), Some(1));
    let receipt = fixture.receipt();
    let commands = receipt["execution"]["commands"].as_array().unwrap();
    assert!(commands.iter().any(|command| command["timed_out"] == true));
    let positions = |argument: &str| {
        commands
            .iter()
            .position(|command| {
                command["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|arg| arg == argument)
            })
            .unwrap()
    };
    assert!(positions("stop") < commands.len() - 2);
    assert_eq!(positions("down"), commands.len() - 1);
    assert!(fixture.output.join("engine/receipt.json").is_file());
}

#[test]
fn build_refuses_existing_outputs_and_nested_source_outputs() {
    let mut fixture = Fixture::new();
    fs::create_dir_all(&fixture.output).unwrap();
    fs::write(fixture.output.join("keep"), "keep").unwrap();
    let output = fixture.run("", 10);
    assert!(!output.status.success());
    assert_eq!(fs::read(fixture.output.join("keep")).unwrap(), b"keep");
    assert!(!fixture.root.path().join("calls.jsonl").exists());
    fixture.output = fixture.source.join("fixture");
    let output = fixture.run("", 10);
    support::rejected(&output, "build directory must not be inside --source-dir");
    assert!(!fixture.output.exists());
}

#[test]
fn build_rejects_symlink_inputs_before_starting_backend() {
    let fixture = Fixture::new();
    symlink(&fixture.spec, fixture.source.join("linked.spec")).unwrap();
    let output = fixture.run("default-retain", 10);
    assert_eq!(output.status.code(), Some(1));
    let receipt = fixture.receipt();
    assert!(
        receipt["execution"]["failure"]
            .as_str()
            .unwrap()
            .contains("symlinks")
    );
    assert!(
        receipt["execution"]["commands"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!fixture.root.path().join("calls.jsonl").exists());
    assert_eq!(receipt["resources_retained"], false);
    let report = support::machine_report(&output);
    assert!(report["next_steps"].get("shell").is_none());
    let clean = report["next_steps"]["clean"].as_array().unwrap();
    let cleaned = fixture
        .command("default-retain")
        .args(clean.iter().skip(1).map(|arg| arg.as_str().unwrap()))
        .arg("--force")
        .output()
        .unwrap();
    support::success(&cleaned);
    assert!(!fixture.output.exists());
    assert!(!fixture.root.path().join("calls.jsonl").exists());
    fs::remove_file(fixture.source.join("linked.spec")).unwrap();
    support::success(&fixture.run("default-retain", 10));
}

#[test]
fn build_inherits_docker_connection_configuration_and_requires_positive_timeout() {
    let fixture = Fixture::new();
    support::success(&fixture.run("default-context", 30));
    let receipt = fixture.receipt();
    assert!(receipt["context"].is_null());
    assert!(
        receipt["execution"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| !command["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == "--context"))
    );
    let fixture = Fixture::new();
    let output = fixture.run("", 0);
    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.output.exists());
}

#[test]
fn backend_success_is_not_engine_result_acceptance() {
    for mode in [
        "no-receipt",
        "failed-receipt",
        "bad-hash",
        "bad-hash-cleanup",
    ] {
        let fixture = Fixture::new();
        let output = fixture.run(mode, 10);
        assert_eq!(output.status.code(), Some(1), "{mode}: {output:?}");
        let receipt = fixture.receipt();
        assert_eq!(receipt["execution"]["success"], mode != "bad-hash-cleanup");
        if mode == "bad-hash-cleanup" {
            assert!(
                receipt["execution"]["cleanup_failure"]
                    .as_str()
                    .unwrap()
                    .contains("19")
            );
        }
        assert_eq!(receipt["success"], false);
        assert!(
            !receipt["engine_validation_error"]
                .as_str()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn timeout_terminates_docker_plugin_descendants_in_the_process_group() {
    let fixture = Fixture::new();
    let output = fixture.run("timeout-descendant", 10);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        fixture.root.path().join("descendant.pid").is_file(),
        "{output:?}; receipt: {}; context stderr: {:?}",
        fixture.receipt(),
        fs::read_to_string(fixture.output.join("host/000-context-identity.stderr.log"))
    );
    let receipt = fixture.receipt();
    let command = receipt["execution"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["timed_out"] == true)
        .unwrap();
    assert_eq!(command["termination"]["method"], "unix-process-group");
    assert_eq!(command["termination"]["reaped"], true);
    assert!(command["termination"]["error"].is_null());
    let marker_deadline = fs::metadata(fixture.root.path().join("descendant.pid"))
        .unwrap()
        .modified()
        .unwrap()
        + std::time::Duration::from_secs(13);
    if let Ok(remaining) = marker_deadline.duration_since(std::time::SystemTime::now()) {
        std::thread::sleep(remaining);
    }
    assert!(!fixture.root.path().join("descendant-survived").exists());
}

#[test]
fn snapshots_are_readable_by_the_worker_without_changing_original_modes() {
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.spec, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(
        fixture.source.join("source.txt"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fs::set_permissions(&fixture.source, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(fixture.source.join("executable"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(
        fixture.source.join("executable"),
        fs::Permissions::from_mode(0o711),
    )
    .unwrap();
    support::success(&fixture.run("", 10));
    let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    for path in ["SPECS/fixture.spec", "SOURCES/source.txt", "engine.py"] {
        assert_eq!(mode(fixture.output.join("input").join(path)), 0o644);
    }
    assert_eq!(mode(fixture.output.join("input/SOURCES/executable")), 0o755);
    assert_eq!(mode(fixture.output.join("input/SOURCES")), 0o755);
    assert_eq!(mode(fixture.output.join("input")), 0o755);
    assert_eq!(mode(fixture.spec), 0o600);
    assert_eq!(mode(fixture.source.join("source.txt")), 0o600);
    assert_eq!(mode(fixture.source.join("executable")), 0o711);
    assert_eq!(mode(fixture.source), 0o700);
}

#[test]
#[cfg(target_os = "linux")]
fn non_utf8_paths_remain_native_for_io_and_display_only_in_receipts() {
    let mut fixture = Fixture::new();
    let name = |bytes: &[u8]| std::ffi::OsString::from_vec(bytes.to_vec());
    let source = fixture.root.path().join(name(b"sources-\xff"));
    fs::rename(&fixture.source, &source).unwrap();
    fixture.source = source;
    let material = name(b"asset-\xfe.txt");
    fs::write(fixture.source.join(&material), "non-UTF-8 named material\n").unwrap();
    let environment = fixture.root.path().join(name(b"environment-\xfd"));
    fs::rename(&fixture.environment, &environment).unwrap();
    fixture.environment = environment;
    let spec_directory = fixture.root.path().join(name(b"spec-directory-\xfc"));
    fs::create_dir(&spec_directory).unwrap();
    let spec = spec_directory.join("fixture.spec");
    fs::rename(&fixture.spec, &spec).unwrap();
    fixture.spec = spec;
    fixture.output = fixture
        .root
        .path()
        .join(name(b"output-\xfb"))
        .join("fixture");
    let output = fixture.run("", 10);
    support::success(&output);
    let receipt = fixture.reported_receipt(&output);
    assert_eq!(receipt["success"], true);
    for field in ["config", "spec", "source_dir"] {
        assert!(receipt[field].as_str().unwrap().contains('\u{fffd}'));
    }
    assert!(
        receipt["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"].as_str().unwrap() == "SOURCES/asset-\u{fffd}.txt")
    );
    assert_eq!(
        fs::read(fixture.output.join("input/SOURCES").join(&material)).unwrap(),
        b"non-UTF-8 named material\n"
    );
    assert_eq!(
        fs::read(fixture.source.join(&material)).unwrap(),
        b"non-UTF-8 named material\n"
    );
}

#[test]
fn clean_requires_confirmation_then_removes_only_receipt_owned_resources() {
    let fixture = Fixture::new();
    support::success(&fixture.run("default-retain", 30));
    let clean = |force: bool| {
        let mut command = fixture.command("");
        command
            .arg("clean")
            .arg("--build-dir")
            .arg(&fixture.output)
            .args(["--format", "toml"]);
        if force {
            command.arg("--force");
        }
        command.output().unwrap()
    };
    let refused = clean(false);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        support::machine_report(&refused)["error"]
            .as_str()
            .unwrap()
            .contains("--force")
    );
    assert!(refused.stderr.is_empty(), "{refused:?}");
    assert!(fixture.output.is_dir());
    let output = clean(true);
    support::success(&output);
    assert!(output.stderr.is_empty(), "{output:?}");
    let report = support::machine_report(&output);
    for (kind, id) in [
        ("container", "abcdef0123456789"),
        ("network", "fedcba9876543210"),
        ("volume", "owned-volume"),
    ] {
        assert_eq!(
            report["removed"][kind]
                .clone()
                .try_into::<Vec<String>>()
                .unwrap(),
            [id],
            "{kind}"
        );
    }
    assert_eq!(
        report["retained_volumes"]
            .clone()
            .try_into::<Vec<String>>()
            .unwrap(),
        ["shared-volume"]
    );
    assert!(!fixture.output.exists());
    assert_eq!(
        fs::read(fixture.source.join("source.txt")).unwrap(),
        b"original material\n"
    );
}

#[test]
fn force_cleanup_still_requires_matching_daemon_and_resource_ownership() {
    for mode in ["wrong-daemon", "wrong-label"] {
        let fixture = Fixture::new();
        support::success(&fixture.run("default-retain", 30));
        if mode == "wrong-daemon" {
            let mut receipt = fixture.receipt();
            receipt["execution"]["details"]["daemon_id"] = "another-daemon".into();
            fs::write(
                fixture.output.join("receipt.json"),
                serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();
        }
        let output = fixture
            .command(mode)
            .arg("clean")
            .arg("--build-dir")
            .arg(&fixture.output)
            .args(["--force", "--format", "toml"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(fixture.output.is_dir());
        let report = support::machine_report(&output);
        assert!(
            report["removed"]["container"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!report["error"].as_str().unwrap().is_empty());
    }
}

#[test]
fn interruption_recovers_evidence_and_obeys_the_selected_removal_policy() {
    use std::{
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    for (signal, mode) in [
        ("-INT", "interrupt"),
        ("-TERM", "interrupt"),
        ("-INT", "interrupt-retain"),
    ] {
        let fixture = Fixture::new();
        let mut child = fixture
            .build_command(mode, 30)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let ready = fixture.root.path().join("engine-ready");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.exists() && Instant::now() < deadline && child.try_wait().unwrap().is_none() {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(
            ready.exists(),
            "engine must have started before cancellation"
        );
        assert!(
            Command::new("/bin/kill")
                .args([signal, &child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                break None;
            }
            thread::sleep(Duration::from_millis(25));
        };
        // Reap a fixture worker even if cancellation regresses to abrupt host exit.
        let _ = Command::new("/bin/kill")
            .args([
                "-KILL",
                "--",
                &format!("-{}", fs::read_to_string(ready).unwrap()),
            ])
            .stderr(Stdio::null())
            .status();
        assert_eq!(status.and_then(|s| s.code()), Some(1));
        let receipt = fixture.receipt();
        assert_eq!(receipt["success"], false);
        let calls = receipt["execution"]["commands"].as_array().unwrap();
        let interrupted = calls.iter().find(|c| c["interrupted"] == true).unwrap();
        assert_eq!(interrupted["argv"][3], "exec");
        assert_eq!(interrupted["timed_out"], false);
        assert!(interrupted["termination"]["error"].is_null());
        assert!(calls.iter().any(|c| c["argv"][3] == "stop"));
        assert_eq!(
            calls
                .iter()
                .any(|c| c["argv"].as_array().unwrap().iter().any(|a| a == "down")),
            mode != "interrupt-retain"
        );
        assert!(fixture.output.join("engine/receipt.json").is_file());
    }
}

#[test]
fn embedded_config_works_outside_a_checkout_and_explicit_config_is_not_replaced() {
    for custom in [false, true] {
        let fixture = Fixture::new();
        let mut command = fixture.build_command("builtin", 30);
        command.current_dir(fixture.root.path());
        let custom_path = fixture.environment.join("advanced.yaml");
        if custom {
            fs::rename(fixture.environment.join("compose.yaml"), &custom_path).unwrap();
            command.arg("--config").arg(&custom_path);
        }
        support::success(&command.output().unwrap());
        let receipt = fixture.receipt();
        assert_eq!(receipt["package"], "fixture");
        assert_eq!(
            receipt["configuration_files"].as_array().unwrap().len(),
            if custom { 1 } else { 4 }
        );
        if custom {
            assert_eq!(
                receipt["config"],
                custom_path
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            );
            assert!(!fixture.output.join(".config").exists());
        } else {
            for file in ["Dockerfile", "compose.yaml", "openruyi.cfg", "target.json"] {
                assert_eq!(
                    fs::read(fixture.output.join(".config").join(file)).unwrap(),
                    fs::read(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("environments/openruyi")
                            .join(file)
                    )
                    .unwrap()
                );
            }
        }
    }
}

#[test]
fn shell_reuses_the_owned_worker_and_stops_it_without_rewriting_the_build_receipt() {
    for mode in ["", "wrong-label", "running-worker", "shell-failure"] {
        let fixture = Fixture::new();
        support::success(&fixture.run("default-retain", 30));
        let before = fs::read(fixture.output.join("receipt.json")).unwrap();
        let lock = fs::File::open(fixture.output.join("receipt.json")).unwrap();
        lock.try_lock().unwrap();
        let calls_before = fs::read(fixture.root.path().join("calls.jsonl")).unwrap();
        let blocked = fixture
            .command(mode)
            .args(["shell", "--build-dir"])
            .arg(&fixture.output)
            .output()
            .unwrap();
        assert_eq!(blocked.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("another shell session"));
        let blocked_clean = fixture
            .command(mode)
            .arg("clean")
            .arg("--build-dir")
            .arg(&fixture.output)
            .args(["--force", "--format", "toml"])
            .output()
            .unwrap();
        assert_eq!(blocked_clean.status.code(), Some(1), "{blocked_clean:?}");
        assert!(
            support::machine_report(&blocked_clean)["error"]
                .as_str()
                .unwrap()
                .contains("another shell or cleanup session")
        );
        assert_eq!(
            fs::read(fixture.root.path().join("calls.jsonl")).unwrap(),
            calls_before
        );
        assert_eq!(
            fs::read(fixture.output.join("receipt.json")).unwrap(),
            before
        );
        assert!(fixture.output.is_dir());
        // Releasing the fixture lease must not depend on the last descriptor closing.
        let _descriptor_copy = lock.try_clone().unwrap();
        lock.unlock().unwrap();
        let result = fixture
            .command(mode)
            .args(["shell", "--build-dir"])
            .arg(&fixture.output)
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(i32::from(!mode.is_empty())),
            "{result:?}"
        );
        assert_eq!(
            fs::read(fixture.output.join("receipt.json")).unwrap(),
            before
        );
        if mode == "shell-failure" {
            let stderr = support::output_text(&result.stderr);
            assert!(
                stderr.contains("[ERROR] shell: exit status: 42"),
                "{stderr}"
            );
        }
        let record = fs::read_dir(fixture.output.join("host"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "json"))
            .unwrap_or_else(|| panic!("mode={mode:?}; missing shell record: {result:?}"));
        let report: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
        let launched = mode.is_empty() || mode == "shell-failure";
        assert_eq!(
            report["exit_code"],
            if launched {
                Value::from(if mode.is_empty() { 0 } else { 42 })
            } else {
                Value::Null
            }
        );
        assert_eq!(
            report["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["argv"][3] == "stop"),
            launched
        );
    }
}

#[test]
fn unsupported_daemon_fails_before_creating_a_worker() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run("windows-daemon", 30).status.code(), Some(1));
    let receipt = fixture.receipt();
    assert!(
        receipt["execution"]["failure"]
            .as_str()
            .unwrap()
            .contains("Linux Docker daemon")
    );
    assert!(!fixture.root.path().join("project").exists());
}

#[test]
fn build_fetches_only_checksum_verified_missing_materials() {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    for matches in [true, false] {
        let fixture = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/archive.tar", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = [0; 4096];
                        assert!(stream.read(&mut request).unwrap() > 0);
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\narchive\n").unwrap();
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        let hash = if matches {
            format!("{:x}", Sha256::digest(b"archive\n"))
        } else {
            "0".repeat(64)
        };
        let spec =
            format!("Name: fixture\nVersion: 1\n#!RemoteAsset:  sha256:{hash}\nSource0: {url}\n");
        fs::write(&fixture.spec, &spec).unwrap();
        let output = fixture
            .build_command("default-retain", 30)
            .output()
            .unwrap();
        server.join().unwrap();
        assert_eq!(
            output.status.success(),
            matches,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_to_string(&fixture.spec).unwrap(), spec);
        if matches {
            assert_eq!(
                fs::read(fixture.source.join("archive.tar")).unwrap(),
                b"archive\n"
            );
            assert_eq!(
                fs::read(fixture.output.join("input/SOURCES/archive.tar")).unwrap(),
                b"archive\n"
            );
        } else {
            assert!(!fixture.source.join("archive.tar").exists());
            assert!(!fixture.root.path().join("calls.jsonl").exists());
        }
    }
}

#[test]
fn repeated_build_preflights_then_reuses_worker_and_preserves_logs() {
    let fixture = Fixture::new();
    support::success(&fixture.run("default-retain", 10));
    let receipt = fs::read(fixture.output.join("receipt.json")).unwrap();
    fs::write(
        fixture.output.join("maintainer.log"),
        "previous failure details",
    )
    .unwrap();
    let calls = fs::read(fixture.root.path().join("calls.jsonl")).unwrap();
    fs::write(
        &fixture.spec,
        "Name: fixture\nVersion: 1\nSource0: missing.patch\n",
    )
    .unwrap();
    let rejected = fixture
        .build_command("default-retain", 10)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert_eq!(
        fs::read(fixture.output.join("receipt.json")).unwrap(),
        receipt
    );
    assert_eq!(
        fs::read(fixture.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    fs::write(fixture.source.join("missing.patch"), "material").unwrap();
    support::success(
        &fixture
            .build_command("default-retain", 10)
            .output()
            .unwrap(),
    );
    let archives: Vec<_> = fs::read_dir(fixture.output.parent().unwrap().join("build-history"))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(archives.len(), 1);
    let previous: Value =
        serde_json::from_slice(&fs::read(archives[0].path().join("receipt.json")).unwrap())
            .unwrap();
    assert_eq!(previous["resources_retained"], false);
    let current = fixture.receipt();
    assert_eq!(
        previous["execution"]["details"],
        current["execution"]["details"]
    );
    let calls = fs::read_to_string(fixture.root.path().join("calls.jsonl")).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("\"create\""))
            .count(),
        1
    );
    assert!(!calls.lines().any(|line| line.contains("\"rm\"")));
    assert_eq!(
        fs::read_to_string(archives[0].path().join("maintainer.log")).unwrap(),
        "previous failure details"
    );
    assert!(fixture.output.join("engine/artifact.rpm").is_file());
    let calls_before = fs::read(fixture.root.path().join("calls.jsonl")).unwrap();
    support::success(
        &fixture
            .command("default-retain")
            .args(["clean", "--build-dir"])
            .arg(archives[0].path())
            .arg("--force")
            .output()
            .unwrap(),
    );
    assert_eq!(
        fs::read(fixture.root.path().join("calls.jsonl")).unwrap(),
        calls_before
    );
    assert!(fixture.output.join("receipt.json").is_file());
}

#[test]
fn prep_reports_stage_without_claiming_a_full_build() {
    let fixture = Fixture::new();
    let output = fixture
        .build_command("default-retain", 10)
        .args(["--stage", "prep"])
        .output()
        .unwrap();
    support::success(&output);
    assert_eq!(fixture.receipt()["stage"], "prep");
    let engine: Value =
        serde_json::from_slice(&fs::read(fixture.output.join("engine/receipt.json")).unwrap())
            .unwrap();
    assert_eq!(engine["target_stage"], "prep");
}

#[test]
fn work_material_inventory_fetch_and_build_share_content_addressed_sources() {
    let fixture = Fixture::new();
    let workspace = fixture.workspace("SPECS", "fixture", "fixture.spec");
    let recipes = workspace.join("recipes");
    let bytes = b"cached archive\n";
    let hash = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes));
    fs::write(recipes.join("SPECS/fixture/fixture.spec"), format!("Name: fixture\nVersion: 1\n#!RemoteAsset:  sha256:{hash}\nSource0: https://example.invalid/archive.tar\n")).unwrap();
    git(&recipes, &["add", "."]);
    git(&recipes, &["commit", "--quiet", "-m", "Declared material"]);
    support::success(
        &fixture
            .command("default-retain")
            .current_dir(&workspace)
            .args(["inspect", "fixture"])
            .output()
            .unwrap(),
    );
    let objects = workspace.join("areas/fixture/sources/.objects");
    fs::create_dir_all(&objects).unwrap();
    fs::write(objects.join(&hash), bytes).unwrap();
    let fetch = fixture
        .command("default-retain")
        .current_dir(&workspace)
        .args([
            "source",
            "fetch",
            "fixture",
            "--offline",
            "--format",
            "toml",
        ])
        .output()
        .unwrap();
    support::success(&fetch);
    assert!(!fixture.root.path().join("calls.jsonl").exists());
    let check = fixture
        .command("default-retain")
        .current_dir(&workspace)
        .args(["check", "fixture", "--materials", "--format", "toml"])
        .output()
        .unwrap();
    let report: toml::Value = toml::from_str(std::str::from_utf8(&check.stdout).unwrap()).unwrap();
    assert_eq!(report["materials"]["valid"].as_bool(), Some(true));
    support::success(
        &fixture
            .command("default-retain")
            .current_dir(&workspace)
            .args([
                "build",
                "fixture",
                "--offline",
                "--context",
                "personalDocker",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(
        fs::read(workspace.join("areas/fixture/build/input/SOURCES/archive.tar")).unwrap(),
        bytes
    );
    let checkout = workspace.join("areas/fixture/recipe/SPECS/fixture/archive.tar");
    fs::write(checkout, "different bytes").unwrap();
    let conflict = fixture
        .command("default-retain")
        .current_dir(&workspace)
        .args(["source", "fetch", "fixture", "--offline"])
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("conflict"));
    assert_eq!(fs::read(objects.join(hash)).unwrap(), bytes);
}

#[test]
fn capability_failure_stops_before_input_copy_or_engine() {
    let fixture = Fixture::new();
    let output = fixture.run("probe-failure", 30);
    assert_eq!(output.status.code(), Some(1));
    let receipt = fixture.receipt();
    assert!(!receipt["success"].as_bool().unwrap());
    let calls = fs::read_to_string(fixture.root.path().join("calls.jsonl")).unwrap();
    assert!(!calls.contains("/input/engine.py"));
    assert!(!fixture.root.path().join("input").exists());
    assert!(calls.contains("stop"));
}

#[test]
fn shell_command_keeps_argv_logs_and_marks_the_environment() {
    let fixture = Fixture::new();
    support::success(&fixture.run("default-retain", 30));
    let output = fixture
        .command("")
        .args(["shell", "--build-dir"])
        .arg(&fixture.output)
        .args(["--", "printf", "%s", "literal; $HOME"])
        .output()
        .unwrap();
    support::success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("literal; $HOME"));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let session: toml::Table =
        toml::from_str(&fs::read_to_string(fixture.output.join("session.toml")).unwrap()).unwrap();
    assert_eq!(
        session["environment_may_have_changed"].as_bool(),
        Some(true)
    );
    assert_eq!(
        session["argv"].as_array().unwrap().last().unwrap().as_str(),
        Some("literal; $HOME")
    );
    let logs = fs::read_dir(fixture.output.join("host"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("command.stdout.log")
        })
        .map(|entry| fs::read_to_string(entry.path()).unwrap())
        .collect::<Vec<_>>();
    assert!(logs.iter().any(|log| log.contains("literal; $HOME")));
    let record = fs::read_dir(fixture.output.join("host"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let report: Value = serde_json::from_slice(&fs::read(record.path()).unwrap()).unwrap();
    assert_eq!(report["exit_code"], 0);
}

#[test]
fn delete_preserves_partial_resource_cleanup_and_unfinished_authoring_work() {
    let fixture = Fixture::new();
    support::success(&fixture.run("default-retain", 30));
    let workspace = tempfile::tempdir().unwrap();
    let area = support::recipe_workspace(
        workspace.path(),
        "review",
        "ed",
        include_str!("../fixtures/ed.spec"),
    );
    fs::write(area.join("ed.toml"), "# unfinished authoring\n").unwrap();
    support::success(&support::run(
        workspace.path(),
        &["open", "review", "--editor=true"],
    ));
    fs::rename(&fixture.output, area.join("build")).unwrap();
    let output = fixture
        .command("remove-network-failure")
        .current_dir(workspace.path())
        .args(["delete", "review", "--force", "--format=toml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty(), "{output:?}");
    let report = support::machine_report(&output);
    let cleanup = &report["cleanup"][0];
    assert_eq!(
        cleanup["removed"]["container"][0].as_str(),
        Some("abcdef0123456789")
    );
    assert!(cleanup["removed"]["network"].as_array().unwrap().is_empty());
    assert_eq!(cleanup["result_removed"].as_bool(), Some(false));
    assert!(report["completed"].as_array().unwrap().is_empty());
    assert!(area.join("recipe/SPECS/ed/ed.spec").is_file());
    assert!(area.join("ed.toml").is_file());
    assert!(area.join("build/receipt.json").is_file());
}

#[test]
fn imported_local_recipe_builds_rebuilds_and_enters_the_same_retained_environment() {
    let mut fixture = Fixture::new();
    let workspace = fixture.root.path().join("local-workspace");
    support::success(
        &fixture
            .command("default-context")
            .args(["init"])
            .arg(&workspace)
            .output()
            .unwrap(),
    );
    fs::copy(&fixture.spec, fixture.source.join("fixture.spec")).unwrap();
    support::success(
        &fixture
            .command("default-context")
            .current_dir(&workspace)
            .args(["new", "trial", "--pkgname=fixture", "--from-dir"])
            .arg(&fixture.source)
            .output()
            .unwrap(),
    );
    fixture.output = workspace.join("work/trial/build");
    for _ in 0..2 {
        let output = fixture
            .command("default-context")
            .current_dir(&workspace)
            .args(["build", "trial", "--format=toml"])
            .output()
            .unwrap();
        support::success(&output);
        assert_eq!(fixture.reported_receipt(&output)["package"], "fixture");
        assert_eq!(
            fs::read(fixture.output.join("input/SOURCES/source.txt")).unwrap(),
            b"original material\n"
        );
    }
    let shell = fixture
        .command("default-context")
        .current_dir(&workspace)
        .args(["shell", "trial", "--", "true"])
        .output()
        .unwrap();
    support::success(&shell);
    assert!(support::output_text(&shell.stdout).contains("mock chroot shell fixture"));
    support::success(
        &fixture
            .command("default-context")
            .current_dir(&workspace)
            .args(["clean", "trial", "--force", "--format=toml"])
            .output()
            .unwrap(),
    );
    assert!(!fixture.output.exists());
    assert!(
        workspace
            .join("work/trial/recipe/SPECS/fixture/fixture.spec")
            .exists()
    );
}

#[test]
fn task_repairs_validates_and_reuses_a_completed_local_task() {
    let fixture = Fixture::new();
    let root = fixture.root.path().join("maintenance");
    fs::create_dir(&root).unwrap();
    let original = include_str!("../fixtures/ed.spec");
    let source = original
        .lines()
        .filter(|line| !line.starts_with("#!RemoteAsset") && !line.starts_with("Source0:"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let source = source.replace(
        "A line-oriented text editor",
        "A line-oriented text editor.",
    );
    let area = support::recipe_workspace(&root, "ed", "ed", &source);
    let repo = root.join("openruyi");
    git(&repo, &["config", "user.name", "Fixture Author"]);
    git(&repo, &["config", "user.email", "fixture@example.org"]);
    fs::write(root.join("plan.toml"), "[[packages]]\nwork='ed'\n").unwrap();
    let run = || {
        fixture
            .command("default-context")
            .current_dir(&root)
            .args(["task", "--plan", "plan.toml", "run", "--format=toml"])
            .output()
            .unwrap()
    };
    fs::write(repo.join("uncommitted-note"), "unrelated work\n").unwrap();
    support::success(
        &fixture
            .command("default-context")
            .current_dir(&root)
            .args(["source", "fetch", "ed"])
            .output()
            .unwrap(),
    );
    fs::rename(repo.join(".git"), repo.join("saved-git")).unwrap();
    let first = run();
    assert!(
        first.status.success(),
        "{first:?}\n{}\n{}",
        fs::read_to_string(root.join(".ruyiconfig/tasks/ed/1.stdout.toml")).unwrap_or_default(),
        fs::read_to_string(root.join(".ruyiconfig/tasks/ed/1.stderr.log")).unwrap_or_default()
    );

    let report = support::machine_report(&first);
    assert_eq!(report["tasks"][0]["phase"].as_str(), Some("validated"));
    let changed = fs::read_to_string(area.join("recipe/SPECS/ed/ed.spec")).unwrap();
    assert!(
        !changed
            .lines()
            .find(|line| line.starts_with("Summary:"))
            .unwrap()
            .ends_with('.')
    );
    assert_eq!(
        fs::read_to_string(repo.join("SPECS/ed/ed.spec")).unwrap(),
        source
    );
    let ready: toml::Value =
        toml::from_str(&fs::read_to_string(report["validated_plan"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(ready["packages"][0]["work"].as_str(), Some("ed"));
    let calls = fs::read(fixture.root.path().join("calls.jsonl")).unwrap();
    let second = run();
    support::success(&second);
    assert_eq!(
        support::machine_report(&second)["tasks"][0]["attempt"],
        report["tasks"][0]["attempt"]
    );
    assert_eq!(
        fs::read(fixture.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    let refreshed = fixture
        .command("default-context")
        .current_dir(&root)
        .args([
            "task",
            "--plan",
            "plan.toml",
            "run",
            "--refresh",
            "--format=toml",
        ])
        .output()
        .unwrap();
    support::success(&refreshed);
    assert!(
        support::machine_report(&refreshed)["tasks"][0]["attempt"]
            .as_integer()
            .unwrap()
            > report["tasks"][0]["attempt"].as_integer().unwrap()
    );
    assert_eq!(
        fs::read(fixture.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    fs::write(area.join("recipe/SPECS/ed/README"), "manual change\n").unwrap();
    let stale = run();
    support::success(&stale);
    let stale = support::machine_report(&stale);
    assert_eq!(stale["tasks"][0]["phase"].as_str(), Some("validated"));
    let ready: toml::Value =
        toml::from_str(&fs::read_to_string(stale["validated_plan"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(ready["packages"][0]["work"].as_str(), Some("ed"));
    assert_eq!(
        fs::read(fixture.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn task_noop_and_failure_stop_without_implicit_rebuilds() {
    for change in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.root.path().join("maintenance");
        fs::create_dir(&root).unwrap();
        let source = include_str!("../fixtures/ed.spec")
            .lines()
            .filter(|line| !line.starts_with("#!RemoteAsset") && !line.starts_with("Source0:"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let source = if change {
            source.replace(
                "Summary:        A line-oriented text editor",
                "Summary:        A line-oriented text editor.",
            )
        } else {
            source
        };
        let area = support::recipe_workspace(&root, "ed", "ed", &source);
        let repo = root.join("openruyi");
        git(&repo, &["config", "user.name", "Fixture Author"]);
        git(&repo, &["config", "user.email", "fixture@example.org"]);
        fs::write(root.join("plan.toml"), "[[packages]]\nwork='ed'\n").unwrap();
        let run = |mode: &str, retry: bool| {
            let mut command = fixture.command(mode);
            command.current_dir(&root).args([
                "task",
                "--plan",
                "plan.toml",
                "run",
                "--format=toml",
            ]);
            if retry {
                command.arg("--retry");
            }
            command.output().unwrap()
        };
        let first = run("default-context-failure", false);
        assert_eq!(first.status.success(), !change, "{first:?}");
        let first = support::machine_report(&first);
        assert_eq!(
            first["tasks"][0]["phase"].as_str(),
            Some(if change { "failed" } else { "unchanged" })
        );
        let spec = fs::read(area.join("recipe/SPECS/ed/ed.spec")).unwrap();
        let calls = fs::read(fixture.root.path().join("calls.jsonl")).ok();
        let second = support::machine_report(&run("default-context-failure", false));
        assert_eq!(second["tasks"][0]["attempt"], first["tasks"][0]["attempt"]);
        assert_eq!(
            fs::read(fixture.root.path().join("calls.jsonl")).ok(),
            calls
        );
        if change {
            let retried = run("default-context", true);
            support::success(&retried);
            assert_eq!(
                support::machine_report(&retried)["tasks"][0]["phase"].as_str(),
                Some("validated")
            );
            assert_eq!(
                fs::read(area.join("recipe/SPECS/ed/ed.spec")).unwrap(),
                spec
            );
        } else {
            assert!(calls.is_none());
        }
        assert_eq!(
            fs::read_to_string(repo.join("SPECS/ed/ed.spec")).unwrap(),
            source
        );
    }
}

#[test]
fn delete_removes_exclusive_images_but_keeps_shared_tags_and_other_containers() {
    let fixture = Fixture::new();
    support::success(&fixture.run("default-retain", 30));
    let workspace = tempfile::tempdir().unwrap();
    let area = support::recipe_workspace(
        workspace.path(),
        "review",
        "ed",
        include_str!("../fixtures/ed.spec"),
    );
    fs::rename(&fixture.output, area.join("build")).unwrap();
    let output = fixture
        .command("owned-images")
        .current_dir(workspace.path())
        .args(["delete", "review", "--force", "--format=toml"])
        .output()
        .unwrap();
    support::success(&output);
    let report = support::machine_report(&output);
    let cleanup = &report["cleanup"][0];
    assert_eq!(
        cleanup["removed"]["image"][0].as_str(),
        Some(format!("sha256:{}", "1".repeat(64)).as_str())
    );
    assert_eq!(cleanup["retained_images"].as_array().unwrap().len(), 2);
    assert!(!area.exists());
    assert!(!area.join("build").exists());
    let calls = fs::read_to_string(fixture.root.path().join("calls.jsonl")).unwrap();
    let image_removals: Vec<Vec<String>> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|args: &Vec<String>| args.windows(2).any(|pair| pair == ["image", "rm"]))
        .collect();
    assert_eq!(image_removals.len(), 1);
    assert!(!image_removals[0].iter().any(|arg| arg == "--force"));
}
