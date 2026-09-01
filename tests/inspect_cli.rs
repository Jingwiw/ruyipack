// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for the `ruyipack inspect` command contract.

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ruyipack-inspect-cli-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create temporary directory");
        Self { path }
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path.join(name);
        fs::write(&path, contents).expect("write temporary SPEC");
        path
    }

    fn entries(&self) -> Vec<PathBuf> {
        let mut entries = fs::read_dir(&self.path)
            .expect("read temporary directory")
            .map(|entry| entry.expect("read directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn ruyipack() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ruyipack"))
}

fn run<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    ruyipack().args(args).output().expect("run ruyipack")
}

fn output_text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("command output is UTF-8")
}

#[test]
fn inspect_prints_the_main_package_view_without_writing() {
    let temp = TempDir::new();
    let spec = temp.write(
        "demo.spec",
        "\
Name:           demo
Version:        1.2.3
Release: %autorelease
Summary: Demo package
License: MIT
URL: https://example.invalid/demo
VCS: https://example.invalid/demo.git
BuildRequires: gcc
%if 1
BuildRequires: make
%else
BuildRequires: ninja-build
%endif

%description
Demo package.

%package devel
Summary: Development files

%description devel
Development files.

%install -a
cat <<'BODY'
Name: generated
Version: 9
BODY
",
    );
    let before_contents = fs::read(&spec).expect("read SPEC before inspection");
    let before_entries = temp.entries();

    let output = ruyipack()
        .current_dir(&temp.path)
        .args([OsStr::new("inspect"), OsStr::new("demo.spec")])
        .output()
        .expect("run ruyipack");

    assert!(
        output.status.success(),
        "status={:?}, stderr={}",
        output.status,
        output_text(&output.stderr)
    );
    assert_eq!(
        output_text(&output.stdout),
        "\
Name: demo
Version: 1.2.3
Release: %autorelease
Summary: Demo package
License: MIT
URL: https://example.invalid/demo
VCS: https://example.invalid/demo.git
BuildRequires: gcc
%if 1
BuildRequires: make
%else
BuildRequires: ninja-build
%endif
"
    );
    assert!(output.stderr.is_empty(), "{}", output_text(&output.stderr));
    assert_eq!(
        fs::read(&spec).expect("read SPEC after inspection"),
        before_contents
    );
    assert_eq!(temp.entries(), before_entries);
}

#[test]
fn inspect_reports_a_recoverable_parser_error_and_succeeds() {
    let temp = TempDir::new();
    let spec = temp.write(
        "missing-subpackage-name.spec",
        "\
Name: demo
Version: 1

%package
Summary: Broken subpackage
",
    );

    let output = run([OsStr::new("inspect"), spec.as_os_str()]);

    assert!(
        output.status.success(),
        "status={:?}, stderr={}",
        output.status,
        output_text(&output.stderr)
    );
    assert_eq!(output_text(&output.stdout), "Name: demo\nVersion: 1\n");
    assert_eq!(
        output_text(&output.stderr),
        "error[rpmspec/E0007] at 4:9: %package requires a subpackage name argument\n"
    );
}

#[test]
fn inspect_rejects_invalid_invocations() {
    let cases = [
        Vec::new(),
        vec![OsString::from("unknown")],
        vec![OsString::from("inspect")],
        vec![
            OsString::from("inspect"),
            OsString::from("demo.spec"),
            OsString::from("extra"),
        ],
    ];

    for args in cases {
        let output = run(&args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "args={args:?}, status={:?}",
            output.status
        );
        assert!(
            output.stdout.is_empty(),
            "args={args:?}, stdout={}",
            output_text(&output.stdout)
        );
        assert!(
            output_text(&output.stderr).contains("Usage: ruyipack inspect <SPEC>"),
            "args={args:?}, stderr={}",
            output_text(&output.stderr)
        );
    }

    let empty_path = run([OsString::from("inspect"), OsString::new()]);
    assert_eq!(empty_path.status.code(), Some(2));
    assert!(empty_path.stdout.is_empty());
    assert!(
        output_text(&empty_path.stderr).contains("SPEC path must not be empty"),
        "{}",
        output_text(&empty_path.stderr)
    );
}

#[test]
fn inspect_rejects_missing_and_non_utf8_inputs() {
    let temp = TempDir::new();
    let missing = temp.path.join("missing.spec");

    let missing_output = run([OsStr::new("inspect"), missing.as_os_str()]);
    assert_eq!(missing_output.status.code(), Some(1));
    assert!(missing_output.stdout.is_empty());
    assert!(
        output_text(&missing_output.stderr)
            .contains(&format!("failed to read {}", missing.display())),
        "{}",
        output_text(&missing_output.stderr)
    );

    let invalid = temp.write("invalid.spec", [0xff]);
    let invalid_output = run([OsStr::new("inspect"), invalid.as_os_str()]);
    assert_eq!(invalid_output.status.code(), Some(1));
    assert!(invalid_output.stdout.is_empty());
    assert!(
        output_text(&invalid_output.stderr)
            .contains(&format!("{} is not UTF-8", invalid.display())),
        "{}",
        output_text(&invalid_output.stderr)
    );
}
