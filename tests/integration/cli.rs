// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Black-box tests for the `RuyiPack` command-line contract.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Output,
};

use serde_json::Value;

use super::support;

use support::{command, json_line, output_text};

fn write_file(directory: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, contents).expect("write test fixture");
    path
}

fn entries(directory: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(directory)
        .expect("read temporary directory")
        .map(|entry| entry.expect("read directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn run<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command().args(args).output().expect("run ruyipack")
}

fn run_json_check(current_dir: &Path, spec: &OsStr) -> Output {
    command()
        .current_dir(current_dir)
        .args([
            OsStr::new("check"),
            spec,
            OsStr::new("--format"),
            OsStr::new("json"),
        ])
        .output()
        .expect("run ruyipack JSON check")
}

#[cfg(target_os = "linux")]
#[test]
fn json_reports_accept_non_utf8_paths_in_success_and_failure() {
    use std::os::unix::ffi::OsStrExt;

    let temp = tempfile::tempdir().unwrap();
    let spec = temp.path().join(OsStr::from_bytes(b"ed-\xff.spec"));
    let manifest = temp.path().join(OsStr::from_bytes(b"ed-\xff.toml"));
    let original = include_str!("../fixtures/ed.spec");
    fs::write(&spec, original).unwrap();
    fs::write(&manifest, include_str!("../../examples/ed/ed.toml")).unwrap();
    for (prefix, input, suffix, exit, pointer) in [
        (&["check"][..], &spec, &[][..], 0, "/input/display_path"),
        (&["inspect"][..], &spec, &[][..], 0, "/input/display_path"),
        (
            &["gen", "ed", "--manifest"][..],
            &manifest,
            &["--check"][..],
            0,
            "/manifest/display_path",
        ),
        (
            &["edit"][..],
            &spec,
            &["--check", "--set", "package.version=2"][..],
            0,
            "/files/0/source",
        ),
        (
            &["edit"][..],
            &spec,
            &["--set", "package.version=1.22.5"][..],
            0,
            "/outcomes/0/path",
        ),
        (
            &["edit"][..],
            &spec,
            &["--check", "--set", "package.version="][..],
            1,
            "/files/0/error/path",
        ),
        (
            &["edit"][..],
            &spec,
            &["--check", "--field", "missing"][..],
            1,
            "/error/path",
        ),
    ] {
        let output = command()
            .args(prefix)
            .arg(input)
            .args(suffix)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(exit),
            "{prefix:?} {suffix:?}: {output:?}"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report.pointer(pointer).and_then(Value::as_str),
            Some(input.to_string_lossy().as_ref())
        );
    }
    support::assert_file(spec, original);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}

fn machine_report(output: &Output) -> Value {
    assert!(output.stderr.is_empty(), "{}", output_text(&output.stderr));
    json_line(output)
}

fn assert_machine_envelope(
    report: &Value,
    display_path: &str,
    sha256: &str,
    status: &str,
    incomplete_reasons: &[&str],
) {
    assert_eq!(report["format_version"], 2);

    let input = &report["input"];
    assert_eq!(input["display_path"], display_path);
    assert_eq!(input["sha256"], sha256);

    let evidence = &report["evidence"];
    assert_eq!(evidence["stage"], "spec-static");
    assert_eq!(evidence["status"], status);
    assert_eq!(
        evidence["incomplete_reasons"],
        serde_json::json!(incomplete_reasons)
    );

    assert_eq!(evidence["tool"]["name"], "ruyipack");
    assert_eq!(evidence["tool"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        evidence["tool"]["revision"],
        if env!("RUYIPACK_BUILD_REVISION").is_empty() {
            serde_json::Value::Null
        } else {
            env!("RUYIPACK_BUILD_REVISION").into()
        }
    );
    assert_eq!(
        evidence["tool"]["dirty"],
        match env!("RUYIPACK_BUILD_DIRTY") {
            "true" => serde_json::json!(true),
            "false" => serde_json::json!(false),
            _ => serde_json::Value::Null,
        }
    );
    assert_eq!(
        evidence["components"],
        serde_json::json!([
            {
                "name": "rpm-spec",
                "version": "0.4.1",
                "repository": "https://github.com/openRuyi-Project/rpm-spec",
                "revision": "2d5139521c8bef5cf61cb9692d75fd0b5282ffd9",
            },
            {
                "name": "rpm-spec-analyzer",
                "version": "0.1.3",
                "repository": "https://github.com/openRuyi-Project/rpm-spec-tool",
                "revision": "9464eccc1f532e28f66e55a47aeaed959172d498",
            },
        ])
    );
    assert_eq!(
        evidence["selected_rules"],
        serde_json::json!([
            {"code": "RPM010", "severity": "deny"},
            {"code": "RPM011", "severity": "deny"},
            {"code": "RPM012", "severity": "deny"},
            {"code": "RPM013", "severity": "deny"},
            {"code": "RPM014", "severity": "deny"},
            {"code": "RPM015", "severity": "deny"},
            {"code": "RPK001", "severity": "deny"},
            {"code": "RPK002", "severity": "deny"},
            {"code": "RPK003", "severity": "deny"},
            {"code": "RPK004", "severity": "deny"},
            {"code": "RPK005", "severity": "warn"},
        ])
    );
}

const COMPLETE_REQUIRED_TAGS: &str = "\
Name: demo
Version: 1
Release: 1
Summary: Demo package
License: MIT
URL: https://example.invalid/demo
";
const COMPLETE_REQUIRED_TAGS_SHA256: &str =
    "8c6dcab3c81d694aa5d28c9b3d833fa6c1b3c875f80dfe01b1093493e52e3730";

const PARSER_WARNING_SPEC: &str = "\
Name: demo
Version: 1
Release: 1
License: MIT
%unknown value
";
const PARSER_WARNING_SPEC_SHA256: &str =
    "baaecaa6a7f2e5071fbf1036b6e7e8bc53c77c10b6bf8abcd1d8578695488704";

const PARSER_ERROR_SPEC: &str = "\
Name: demo
Version: 1
Release: 1
Summary: Demo package
License: MIT

%package
Summary: Broken subpackage
";
const PARSER_ERROR_SPEC_SHA256: &str =
    "e5bf39331ae71060d336c183c55ebf29da41b085e2205c6c20adfb254bead260";

#[test]
fn check_ignores_directory_lint_config() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let spec = write_file(temp.path(), "demo.spec", COMPLETE_REQUIRED_TAGS);
    write_file(
        temp.path(),
        ".rpmspec.toml",
        "\
[lints]
RPM001 = \"deny\"
",
    );
    let before_contents = fs::read(&spec).expect("read SPEC before check");
    let before_entries = entries(temp.path());

    let output = command()
        .current_dir(temp.path())
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
    assert_eq!(entries(temp.path()), before_entries);
}

#[test]
fn empty_spec_reports_six_missing_tags() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    write_file(temp.path(), "empty.spec", "");
    write_file(
        temp.path(),
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
    let before_entries = entries(temp.path());

    let output = command()
        .current_dir(temp.path())
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
    assert_eq!(entries(temp.path()), before_entries);
}

#[test]
fn check_continues_after_a_parser_warning() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let complete_spec = write_file(
        temp.path(),
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
        format!(
            "{}:7:1: warning[rpmspec/W0002]: line not recognized\n",
            complete_spec.display()
        )
    );

    let spec = write_file(
        temp.path(),
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
{0}:6:1: warning[rpmspec/W0002]: line not recognized
{0}:1:1: error[RPM015]: spec is missing the URL: tag
",
            spec.display()
        )
    );
}

#[test]
fn check_stops_tag_checks_when_the_parser_reports_an_error() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let spec = write_file(temp.path(), "parser-error.spec", PARSER_ERROR_SPEC);

    let output = run([OsStr::new("check"), spec.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert_eq!(
        output_text(&output.stderr),
        format!(
            "{0}:7:9: error[rpmspec/E0007]: %package requires a subpackage name argument\n{0}: error: check incomplete because the SPEC parser reported an error\n",
            spec.display()
        )
    );
}

#[test]
fn check_json_reports_pass_with_input_identity() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    write_file(temp.path(), "demo.spec", COMPLETE_REQUIRED_TAGS);

    let first = run_json_check(temp.path(), OsStr::new("demo.spec"));

    assert_eq!(first.status.code(), Some(0));
    let first_report = machine_report(&first);
    assert_machine_envelope(
        &first_report,
        "demo.spec",
        COMPLETE_REQUIRED_TAGS_SHA256,
        "pass",
        &[],
    );
    assert_eq!(first_report["parser_diagnostics"], serde_json::json!([]));
    assert_eq!(first_report["findings"], serde_json::json!([]));
}

#[test]
fn check_json_keeps_parser_warning_and_orders_findings() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    write_file(temp.path(), "warning.spec", PARSER_WARNING_SPEC);

    let first = run_json_check(temp.path(), OsStr::new("warning.spec"));

    assert_eq!(first.status.code(), Some(1));
    let report = machine_report(&first);
    assert_machine_envelope(
        &report,
        "warning.spec",
        PARSER_WARNING_SPEC_SHA256,
        "fail",
        &[],
    );
    assert_eq!(
        report["parser_diagnostics"],
        serde_json::json!([{
            "severity": "warning",
            "code": "rpmspec/W0002",
            "span": {
                "start_byte": 46,
                "end_byte": 60,
                "start_line": 5,
                "start_column": 1,
                "end_line": 5,
                "end_column": 15,
            },
            "message": "line not recognized",
            "notes": [],
        }])
    );

    let findings = report["findings"].as_array().expect("findings is an array");
    let missing_tags = [("RPM014", "Summary"), ("RPM015", "URL")];
    assert_eq!(findings.len(), missing_tags.len());
    for (finding, (code, tag)) in findings.iter().zip(missing_tags) {
        assert_eq!(finding["producer"], "rpm-spec-analyzer");
        assert_eq!(finding["code"], code);
        assert_eq!(finding["severity"], "deny");
        assert_eq!(
            finding["message"],
            format!("spec is missing the {tag}: tag")
        );
        assert_eq!(
            finding["span"],
            serde_json::json!({
                "start_byte": 0,
                "end_byte": 61,
                "start_line": 1,
                "start_column": 1,
                "end_line": 6,
                "end_column": 1,
            })
        );
    }
}

#[test]
fn check_json_reports_parser_error_as_incomplete() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    write_file(temp.path(), "parser-error.spec", PARSER_ERROR_SPEC);

    let output = run_json_check(temp.path(), OsStr::new("parser-error.spec"));

    assert_eq!(output.status.code(), Some(1));
    let report = machine_report(&output);
    assert_machine_envelope(
        &report,
        "parser-error.spec",
        PARSER_ERROR_SPEC_SHA256,
        "incomplete",
        &["parser-error"],
    );
    assert_eq!(report["findings"], serde_json::json!([]));
    assert_eq!(
        report["parser_diagnostics"],
        serde_json::json!([{
            "severity": "error",
            "code": "rpmspec/E0007",
            "span": {
                "start_byte": 77,
                "end_byte": 77,
                "start_line": 7,
                "start_column": 9,
                "end_line": 7,
                "end_column": 9,
            },
            "message": "%package requires a subpackage name argument",
            "notes": [],
        }])
    );
}

fn assert_read_errors(subcommand: &str) {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let missing = temp.path().join("missing.spec");
    let invalid = write_file(temp.path(), "invalid.spec", [0xff]);
    for (path, message) in [
        (&missing, format!("failed to read {}", missing.display())),
        (&invalid, format!("{} is not UTF-8", invalid.display())),
    ] {
        let output = run([OsStr::new(subcommand), path.as_os_str()]);
        assert_eq!(output.status.code(), Some(1), "{subcommand}: {output:?}");
        assert!(output.stdout.is_empty(), "{subcommand}: {output:?}");
        assert!(
            output_text(&output.stderr).contains(&message),
            "{subcommand}: {}",
            output_text(&output.stderr)
        );
    }
}

#[test]
fn report_commands_reject_missing_and_non_utf8_inputs() {
    for command in ["check", "inspect"] {
        assert_read_errors(command);
    }
}

#[test]
fn input_failures_share_machine_error_shape_across_commands() {
    let temp = tempfile::tempdir().unwrap();
    let invalid = write_file(temp.path(), "invalid.spec", [0xff]);
    for path in [invalid.clone(), temp.path().join("missing.spec")] {
        for args in [
            vec!["check"],
            vec!["inspect"],
            vec!["edit", "--set", "package.version=2", "--check"],
            vec!["source-hash"],
            vec!["verify-sources"],
            vec!["gen", "demo", "--check", "--manifest"],
        ] {
            let output = command()
                .args(&args)
                .arg(&path)
                .args(["--format", "json"])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let report = json_line(&output);
            assert_eq!(report["valid"], false);
            assert_eq!(report["error"]["code"], "input-read", "{args:?}: {report}");
            assert!(!report["error"]["message"].as_str().unwrap().is_empty());
            let subject = match args[0] {
                "gen" => &report["manifest"],
                "edit" => {
                    let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    assert_eq!(report["error"]["path"], resolved.to_string_lossy().as_ref());
                    assert_eq!(
                        report["error"]["selected_fields"],
                        serde_json::json!(["package.version"])
                    );
                    continue;
                }
                _ => &report["input"],
            };
            assert_eq!(subject["display_path"], path.to_string_lossy().as_ref());
            assert!(subject["sha256"].is_null());
        }
    }
    assert_eq!(fs::read(&invalid).unwrap(), [0xff]);
    assert_eq!(entries(temp.path()), [invalid]);
}

#[test]
fn inspect_prints_the_main_package_view_without_writing() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let spec = write_file(
        temp.path(),
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
    let before_entries = entries(temp.path());

    let output = command()
        .current_dir(temp.path())
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
    assert_eq!(entries(temp.path()), before_entries);
}

#[test]
fn inspect_reports_a_recoverable_parser_error_and_succeeds() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let spec = write_file(
        temp.path(),
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
        format!(
            "{}:4:9: error[rpmspec/E0007]: %package requires a subpackage name argument\n",
            spec.display()
        )
    );
}

#[test]
fn cli_rejects_invalid_invocations() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "Usage: ruyipack <COMMAND>"),
        (&["unknown"], "error: unrecognized subcommand 'unknown'"),
        (&["inspect"], "Usage: ruyipack inspect <SPEC>"),
        (&["check"], "Usage: ruyipack check <SPEC>"),
        (
            &["check", "demo.spec", "extra"],
            "error: unexpected argument 'extra' found",
        ),
        (
            &["inspect", "demo.spec", "extra"],
            "error: unexpected argument 'extra' found",
        ),
        (
            &["inspect", ""],
            "a value is required for '<SPEC>' but none was supplied",
        ),
    ];

    for &(args, expected) in cases {
        let output = run(args);
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
}

#[test]
fn cli_prints_standard_help_and_version() {
    let commands = ["init", "gen", "inspect", "check", "source-hash", "edit"];
    let help = run(["--help"]);
    super::support::quiet_success(&help);
    let text = output_text(&help.stdout);
    for name in commands {
        assert!(
            text.lines()
                .any(|line| line.split_whitespace().next() == Some(name)),
            "missing command {name}: {text}"
        );
    }
    // Check product options, not Clap's wording or help paragraph layout.
    for (name, options) in [
        ("inspect", &["--format"] as &[_]),
        ("check", &["--format"]),
        ("gen", &["--format", "--check", "--offline"]),
        ("edit", &["--set", "--field", "--check", "--hash-source"]),
    ] {
        let help = run([name, "--help"]);
        super::support::quiet_success(&help);
        for option in options {
            assert!(
                output_text(&help.stdout).contains(option),
                "{name}: missing {option}"
            );
        }
    }
    let version = run(["--version"]);
    super::support::quiet_success(&version);
    assert_eq!(
        output_text(&version.stdout),
        format!("ruyipack {}\n", env!("CARGO_PKG_VERSION"))
    );
}
