// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for the RuyiPack command-line contract.

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
        let path = std::env::temp_dir().join(format!("ruyipack-cli-{}-{id}", std::process::id()));
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

const COMPLETE_REQUIRED_TAGS: &str = "\
Name: demo
Version: 1
Release: 1
Summary: Demo package
License: MIT
URL: https://example.invalid/demo
";

#[test]
fn check_accepts_the_six_required_tags_without_running_other_rules() {
    let temp = TempDir::new();
    let spec = temp.write("demo.spec", COMPLETE_REQUIRED_TAGS);
    temp.write(
        ".rpmspec.toml",
        "\
[lints]
RPM001 = \"deny\"
",
    );
    let before_contents = fs::read(&spec).expect("read SPEC before check");
    let before_entries = temp.entries();

    let output = ruyipack()
        .current_dir(&temp.path)
        .args([OsStr::new("check"), OsStr::new("demo.spec")])
        .output()
        .expect("run ruyipack");

    assert!(
        output.status.success(),
        "status={:?}, stderr={}",
        output.status,
        output_text(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert!(output.stderr.is_empty(), "{}", output_text(&output.stderr));
    assert_eq!(
        fs::read(&spec).expect("read SPEC after check"),
        before_contents
    );
    assert_eq!(temp.entries(), before_entries);
}

#[test]
fn check_reports_exactly_the_six_required_tag_rules() {
    let temp = TempDir::new();
    temp.write("empty.spec", "");
    temp.write(
        ".rpmspec.toml",
        "\
[lints]
RPM010 = \"allow\"
RPM011 = \"allow\"
RPM012 = \"allow\"
RPM013 = \"allow\"
RPM014 = \"allow\"
RPM015 = \"allow\"
",
    );
    let before_entries = temp.entries();

    let output = ruyipack()
        .current_dir(&temp.path)
        .args([OsStr::new("check"), OsStr::new("empty.spec")])
        .output()
        .expect("run ruyipack");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert_eq!(
        output_text(&output.stderr),
        "\
empty.spec:1:1: error[RPM010]: spec is missing the Name: tag
empty.spec:1:1: error[RPM011]: spec is missing the Version: tag
empty.spec:1:1: error[RPM012]: spec is missing the Release: tag
empty.spec:1:1: error[RPM013]: spec is missing the License: tag
empty.spec:1:1: error[RPM014]: spec is missing the Summary: tag
empty.spec:1:1: error[RPM015]: spec is missing the URL: tag
"
    );
    assert_eq!(temp.entries(), before_entries);
}

#[test]
fn check_continues_after_a_parser_warning() {
    let temp = TempDir::new();
    let complete_spec = temp.write(
        "complete-warning.spec",
        format!("{COMPLETE_REQUIRED_TAGS}%unknown value\n"),
    );
    let complete_output = run([OsStr::new("check"), complete_spec.as_os_str()]);

    assert!(
        complete_output.status.success(),
        "status={:?}, stderr={}",
        complete_output.status,
        output_text(&complete_output.stderr)
    );
    assert!(
        complete_output.stdout.is_empty(),
        "{}",
        output_text(&complete_output.stdout)
    );
    assert_eq!(
        output_text(&complete_output.stderr),
        "warning[rpmspec/W0002] at 7:1: line not recognized\n"
    );

    let spec = temp.write(
        "warning.spec",
        "\
Name: demo
Version: 1
Release: 1
Summary: Demo package
License: MIT
%unknown value
",
    );

    let output = run([OsStr::new("check"), spec.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert_eq!(
        output_text(&output.stderr),
        format!(
            "\
warning[rpmspec/W0002] at 6:1: line not recognized
{}:1:1: error[RPM015]: spec is missing the URL: tag
",
            spec.display()
        )
    );
}

#[test]
fn check_stops_tag_checks_when_the_parser_reports_an_error() {
    let temp = TempDir::new();
    let spec = temp.write(
        "parser-error.spec",
        "\
Name: demo
Version: 1
Release: 1
Summary: Demo package
License: MIT

%package
Summary: Broken subpackage
",
    );

    let output = run([OsStr::new("check"), spec.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert_eq!(
        output_text(&output.stderr),
        "\
error[rpmspec/E0007] at 7:9: %package requires a subpackage name argument
error: check incomplete because the SPEC parser reported an error
"
    );
}

#[test]
fn check_rejects_missing_and_non_utf8_inputs() {
    let temp = TempDir::new();
    let missing = temp.path.join("missing.spec");

    let missing_output = run([OsStr::new("check"), missing.as_os_str()]);
    assert_eq!(missing_output.status.code(), Some(1));
    assert!(missing_output.stdout.is_empty());
    assert!(
        output_text(&missing_output.stderr)
            .contains(&format!("failed to read {}", missing.display())),
        "{}",
        output_text(&missing_output.stderr)
    );

    let invalid = temp.write("invalid.spec", [0xff]);
    let invalid_output = run([OsStr::new("check"), invalid.as_os_str()]);
    assert_eq!(invalid_output.status.code(), Some(1));
    assert!(invalid_output.stdout.is_empty());
    assert!(
        output_text(&invalid_output.stderr)
            .contains(&format!("{} is not UTF-8", invalid.display())),
        "{}",
        output_text(&invalid_output.stderr)
    );
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
fn cli_rejects_invalid_invocations() {
    let cases = [
        (Vec::new(), "Usage: ruyipack <COMMAND>"),
        (
            vec![OsString::from("unknown")],
            "error: unrecognized subcommand 'unknown'",
        ),
        (
            vec![OsString::from("inspect")],
            "Usage: ruyipack inspect <SPEC>",
        ),
        (
            vec![OsString::from("check")],
            "Usage: ruyipack check <SPEC>",
        ),
        (
            vec![
                OsString::from("check"),
                OsString::from("demo.spec"),
                OsString::from("extra"),
            ],
            "error: unexpected argument 'extra' found",
        ),
        (
            vec![
                OsString::from("inspect"),
                OsString::from("demo.spec"),
                OsString::from("extra"),
            ],
            "error: unexpected argument 'extra' found",
        ),
    ];

    for (args, expected) in cases {
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
            output_text(&output.stderr).contains(expected),
            "args={args:?}, stderr={}",
            output_text(&output.stderr)
        );
    }

    let empty_path = run([OsString::from("inspect"), OsString::new()]);
    assert_eq!(empty_path.status.code(), Some(2));
    assert!(empty_path.stdout.is_empty());
    assert!(
        output_text(&empty_path.stderr)
            .contains("a value is required for '<SPEC>' but none was supplied"),
        "{}",
        output_text(&empty_path.stderr)
    );
}

#[test]
fn cli_prints_standard_help_and_version() {
    let help = run([OsStr::new("--help")]);
    assert!(help.status.success());
    assert_eq!(
        output_text(&help.stdout),
        "\
Rust tooling for openRuyi RPM package workflows

Usage: ruyipack <COMMAND>

Commands:
  check    Checks required main-package tag presence in an RPM SPEC file
  inspect  Prints the normalized main-package tags from an RPM SPEC file
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
"
    );
    assert!(help.stderr.is_empty(), "{}", output_text(&help.stderr));

    let inspect_help = run([OsStr::new("inspect"), OsStr::new("--help")]);
    assert!(inspect_help.status.success());
    assert_eq!(
        output_text(&inspect_help.stdout),
        "\
Prints the normalized main-package tags from an RPM SPEC file

Usage: ruyipack inspect <SPEC>

Arguments:
  <SPEC>  RPM SPEC file to inspect

Options:
  -h, --help  Print help
"
    );
    assert!(
        inspect_help.stderr.is_empty(),
        "{}",
        output_text(&inspect_help.stderr)
    );

    let check_help = run([OsStr::new("check"), OsStr::new("--help")]);
    assert!(check_help.status.success());
    assert_eq!(
        output_text(&check_help.stdout),
        "\
Checks required main-package tag presence in an RPM SPEC file

Usage: ruyipack check <SPEC>

Arguments:
  <SPEC>  RPM SPEC file to check

Options:
  -h, --help  Print help
"
    );
    assert!(
        check_help.stderr.is_empty(),
        "{}",
        output_text(&check_help.stderr)
    );

    let version = run([OsStr::new("--version")]);
    assert!(version.status.success());
    assert_eq!(
        output_text(&version.stdout),
        format!("ruyipack {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        version.stderr.is_empty(),
        "{}",
        output_text(&version.stderr)
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
