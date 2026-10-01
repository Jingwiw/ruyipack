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

use toml::Value;

use super::support;

use support::{command, output_text};

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

fn run_toml_check(current_dir: &Path, spec: &OsStr) -> Output {
    support::run(
        current_dir,
        &[
            OsStr::new("check"),
            OsStr::new("--spec"),
            spec,
            OsStr::new("--format"),
            OsStr::new("toml"),
        ],
    )
}

#[cfg(target_os = "linux")]
#[test]
fn toml_reports_accept_non_utf8_paths_in_success_and_failure() {
    use std::os::unix::ffi::OsStrExt;

    let temp = tempfile::tempdir().unwrap();
    let spec = temp.path().join(OsStr::from_bytes(b"ed-\xff.spec"));
    let manifest = temp.path().join(OsStr::from_bytes(b"ed-\xff.toml"));
    let original = include_str!("../fixtures/ed.spec");
    fs::write(&spec, original).unwrap();
    fs::write(&manifest, include_str!("../../examples/ed/ed.toml")).unwrap();
    for (prefix, input, suffix, exit, pointer) in [
        (
            &["check", "--spec"][..],
            &spec,
            &[][..],
            0,
            "/input/display_path",
        ),
        (
            &["inspect", "--spec"][..],
            &spec,
            &[][..],
            0,
            "/input/display_path",
        ),
        (
            &["check", "--manifest"][..],
            &manifest,
            &[][..],
            0,
            "/input/display_path",
        ),
        (
            &["edit", "--spec"][..],
            &spec,
            &["--check"][..],
            0,
            "/files/0/source",
        ),
        (
            &["edit", "--spec"][..],
            &spec,
            &["--check", "--set", "missing=value"][..],
            1,
            "/error/path",
        ),
    ] {
        let output = command()
            .args(prefix)
            .arg(input)
            .args(suffix)
            .args(["--format", "toml"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(exit),
            "{prefix:?} {suffix:?}: {output:?}"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        let report = super::support::machine_report(&output);
        assert_eq!(
            pointer
                .trim_start_matches('/')
                .split('/')
                .try_fold(&report, |value, part| {
                    if let Some(array) = value.as_array() {
                        array.get(part.parse::<usize>().ok()?)
                    } else {
                        value.get(part)
                    }
                })
                .and_then(Value::as_str),
            Some(input.to_string_lossy().as_ref())
        );
    }
    // Display paths are lossy text, not a persistence encoding. A non-UTF-8
    // SPEC stem cannot name a TOML stage; failure must still be a valid report.
    let staged = command()
        .args(["edit", "--spec"])
        .arg(&spec)
        .args(["--set", "package.version=2", "--apply", "--format", "toml"])
        .output()
        .unwrap();
    assert_eq!(staged.status.code(), Some(1), "{staged:?}");
    assert_eq!(machine_report(&staged)["success"].as_bool(), Some(false));
    fs::write(&manifest, "not a manifest").unwrap();
    let output = command()
        .args(["check", "--manifest"])
        .arg(&manifest)
        .args(["--format", "toml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report = machine_report(&output);
    assert_eq!(report["error"]["code"].as_str(), Some("invalid-manifest"));
    assert_eq!(
        report["input"]["display_path"].as_str(),
        Some(manifest.to_string_lossy().as_ref())
    );
    support::assert_file(spec, original);
}

fn machine_report(output: &Output) -> Value {
    assert!(output.stderr.is_empty(), "{}", output_text(&output.stderr));
    support::machine_report(output)
}

fn assert_machine_envelope(
    report: &Value,
    display_path: &str,
    sha256: &str,
    status: &str,
    incomplete_reasons: &[&str],
) {
    assert_eq!(report["format_version"].as_integer(), Some(2));

    let input = &report["input"];
    assert_eq!(input["display_path"].as_str(), Some(display_path));
    assert_eq!(input["sha256"].as_str(), Some(sha256));

    let evidence = &report["evidence"];
    assert_eq!(evidence["stage"].as_str(), Some("spec-static"));
    assert_eq!(evidence["status"].as_str(), Some(status));
    assert_eq!(
        evidence["incomplete_reasons"],
        toml::Value::try_from(incomplete_reasons).unwrap()
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

    let output = support::run(
        temp.path(),
        &[
            OsStr::new("check"),
            OsStr::new("--spec"),
            OsStr::new("demo.spec"),
        ],
    );

    support::success(&output);
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

    let output = support::run(
        temp.path(),
        &[
            OsStr::new("check"),
            OsStr::new("--spec"),
            OsStr::new("empty.spec"),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert_eq!(
        output_text(&output.stderr),
        "\
[INFO] empty.spec: checking SPEC
[ERROR] spec[1:1] [RPM010]: spec is missing the Name: tag
[ERROR] spec[1:1] [RPM011]: spec is missing the Version: tag
[ERROR] spec[1:1] [RPM012]: spec is missing the Release: tag
[ERROR] spec[1:1] [RPM013]: spec is missing the License: tag
[ERROR] spec[1:1] [RPM014]: spec is missing the Summary: tag
[ERROR] spec[1:1] [RPM015]: spec is missing the URL: tag
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
    let complete_output = support::run(
        temp.path(),
        &[
            OsStr::new("check"),
            OsStr::new("--spec"),
            complete_spec.as_os_str(),
        ],
    );

    support::success(&complete_output);
    assert!(
        complete_output.stdout.is_empty(),
        "{}",
        output_text(&complete_output.stdout)
    );
    assert_eq!(
        output_text(&complete_output.stderr),
        format!(
            "[INFO] {}: checking SPEC\n[WARN] spec[7:1] [rpmspec/W0002]: line not recognized\n",
            "complete-warning.spec"
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

    let output = support::run(
        temp.path(),
        &[OsStr::new("check"), OsStr::new("--spec"), spec.as_os_str()],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert_eq!(
        output_text(&output.stderr),
        format!(
            "\
[INFO] {0}: checking SPEC
[WARN] spec[6:1] [rpmspec/W0002]: line not recognized
[ERROR] spec[1:1] [RPM015]: spec is missing the URL: tag
",
            "warning.spec"
        )
    );
}

#[test]
fn check_stops_tag_checks_when_the_parser_reports_an_error() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let spec = write_file(temp.path(), "parser-error.spec", PARSER_ERROR_SPEC);

    let output = support::run(
        temp.path(),
        &[OsStr::new("check"), OsStr::new("--spec"), spec.as_os_str()],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", output_text(&output.stdout));
    assert_eq!(
        output_text(&output.stderr),
        format!(
            "[INFO] {0}: checking SPEC\n[ERROR] spec[7:9] [rpmspec/E0007]: %package requires a subpackage name argument\n[ERROR] check incomplete because the SPEC parser reported an error\n",
            "parser-error.spec"
        )
    );
}

#[test]
fn check_toml_reports_pass_with_input_identity() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    write_file(temp.path(), "demo.spec", COMPLETE_REQUIRED_TAGS);

    let first = run_toml_check(temp.path(), OsStr::new("demo.spec"));

    assert_eq!(first.status.code(), Some(0));
    let first_report = machine_report(&first);
    assert_machine_envelope(
        &first_report,
        "demo.spec",
        COMPLETE_REQUIRED_TAGS_SHA256,
        "pass",
        &[],
    );
    assert_eq!(
        first_report["parser_diagnostics"],
        toml::Value::Array(vec![])
    );
    assert_eq!(first_report["findings"], toml::Value::Array(vec![]));
}

#[test]
fn check_toml_keeps_parser_warning_and_orders_findings() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    write_file(temp.path(), "warning.spec", PARSER_WARNING_SPEC);

    let first = run_toml_check(temp.path(), OsStr::new("warning.spec"));

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
        toml::Value::Array(vec![toml::Value::Table(toml::toml! {
            "severity" = "warning"
            "code" = "rpmspec/W0002"
            "span" = { "start_byte" = 46, "end_byte" = 60, "start_line" = 5, "start_column" = 1, "end_line" = 5, "end_column" = 15 }
            "message" = "line not recognized"
            "notes" = []
        })])
    );

    let findings = report["findings"].as_array().expect("findings is an array");
    let missing_tags = [("RPM014", "Summary"), ("RPM015", "URL")];
    assert_eq!(findings.len(), missing_tags.len());
    for (finding, (code, tag)) in findings.iter().zip(missing_tags) {
        assert_eq!(finding["producer"].as_str(), Some("rpm-spec-analyzer"));
        assert_eq!(finding["code"].as_str(), Some(code));
        assert_eq!(finding["severity"].as_str(), Some("deny"));
        assert_eq!(
            finding["message"].as_str(),
            Some((format!("spec is missing the {tag}: tag")).as_str())
        );
        assert_eq!(
            finding["span"],
            toml::Value::Table(toml::toml! {
                "start_byte" = 0
                "end_byte" = 61
                "start_line" = 1
                "start_column" = 1
                "end_line" = 6
                "end_column" = 1
            })
        );
    }
}

#[test]
fn check_toml_reports_parser_error_as_incomplete() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    write_file(temp.path(), "parser-error.spec", PARSER_ERROR_SPEC);

    let output = run_toml_check(temp.path(), OsStr::new("parser-error.spec"));

    assert_eq!(output.status.code(), Some(1));
    let report = machine_report(&output);
    assert_machine_envelope(
        &report,
        "parser-error.spec",
        PARSER_ERROR_SPEC_SHA256,
        "incomplete",
        &["parser-error"],
    );
    assert_eq!(
        report["evidence"]["source_uncertainty"].as_str(),
        Some("parser errors prevent Source resolution")
    );
    assert_eq!(report["findings"], toml::Value::Array(vec![]));
    assert_eq!(
        report["parser_diagnostics"],
        toml::Value::Array(vec![toml::Value::Table(toml::toml! {
            "severity" = "error"
            "code" = "rpmspec/E0007"
            "span" = { "start_byte" = 77, "end_byte" = 77, "start_line" = 7, "start_column" = 9, "end_line" = 7, "end_column" = 9 }
            "message" = "%package requires a subpackage name argument"
            "notes" = []
        })])
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
        let output = run([
            OsStr::new(subcommand),
            OsStr::new("--spec"),
            path.as_os_str(),
        ]);
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
            vec!["check", "--spec"],
            vec!["inspect", "--spec"],
            vec!["edit", "--set", "package.version=2", "--check", "--spec"],
            vec!["source", "hash", "--spec"],
            vec!["source", "verify", "--spec"],
        ] {
            let output = command()
                .args(&args)
                .arg(&path)
                .args(["--format", "toml"])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let report = machine_report(&output);
            if args[0] == "edit" {
                assert!(report.get("valid").is_none());
                assert_eq!(report["success"].as_bool(), Some(false));
            } else {
                assert_eq!(report["valid"].as_bool(), Some(false));
            }
            assert_eq!(
                report["error"]["code"].as_str(),
                Some("input-read"),
                "{args:?}: {report}"
            );
            assert!(!report["error"]["message"].as_str().unwrap().is_empty());
            let subject = match args[0] {
                "edit" => {
                    let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    assert_eq!(
                        report["error"]["path"].as_str(),
                        Some(resolved.to_string_lossy().as_ref())
                    );
                    assert_eq!(
                        report["error"]["selected_fields"],
                        toml::Value::Array(vec![toml::Value::from("package.version")])
                    );
                    continue;
                }
                _ => &report["input"],
            };
            assert_eq!(
                subject["display_path"].as_str(),
                Some(path.to_string_lossy().as_ref())
            );
            assert!(subject.get("sha256").is_none());
        }
    }
    assert_eq!(fs::read(&invalid).unwrap(), [0xff]);
    assert_eq!(entries(temp.path()), [invalid]);
}

#[test]
fn generation_input_errors_keep_manifest_identity_without_derived_files() {
    for exists in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let work = support::authoring_workspace(
            directory.path(),
            "review",
            "ed",
            include_str!("../../examples/ed/ed.toml"),
        );
        let path = work.canonicalize().unwrap().join("ed.toml");
        if exists {
            fs::write(&path, [0xff]).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        let output = command()
            .current_dir(directory.path())
            .args(["gen", "review", "--check", "--format", "toml"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let report = machine_report(&output);
        assert!(report.get("valid").is_none());
        assert_eq!(report["success"].as_bool(), Some(false));
        assert_eq!(
            report["error"]["code"].as_str(),
            Some("input-read"),
            "{report}"
        );
        assert!(
            report["error"]["message"]
                .as_str()
                .unwrap()
                .contains(path.to_str().unwrap()),
            "{report}"
        );
        assert_eq!(
            report["input"]["display_path"].as_str(),
            Some(path.to_string_lossy().as_ref())
        );
        assert!(report["input"].get("sha256").is_none());
        assert!(!work.join("ed.resolved.toml").exists());
        assert!(!work.join("stage").exists());
        if exists {
            assert_eq!(fs::read(path).unwrap(), [0xff]);
        }
    }
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

    let output = support::run(
        temp.path(),
        &[
            OsStr::new("inspect"),
            OsStr::new("--spec"),
            OsStr::new("demo.spec"),
        ],
    );

    support::success(&output);
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

    let output = support::run(
        temp.path(),
        &[
            OsStr::new("inspect"),
            OsStr::new("--spec"),
            spec.as_os_str(),
        ],
    );

    support::success(&output);
    assert_eq!(output_text(&output.stdout), "Name: demo\nVersion: 1\n");
    assert_eq!(
        output_text(&output.stderr),
        format!(
            "[INFO] {}: inspecting SPEC\n[ERROR] spec[4:9] [rpmspec/E0007]: %package requires a subpackage name argument\n",
            "missing-subpackage-name.spec"
        )
    );
}

#[test]
fn cli_rejects_invalid_invocations() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "Usage: ruyipack <COMMAND>"),
        (&["unknown"], "error: unrecognized subcommand 'unknown'"),
        (&["inspect"], "Usage: ruyipack inspect"),
        (&["check"], "Usage: ruyipack check"),
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
            "a value is required for '[WORK]' but none was supplied",
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
