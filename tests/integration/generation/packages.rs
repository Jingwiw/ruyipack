// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Main-package and subpackage names, dependencies, and file ownership.

use super::{MANIFEST, assert_file, quiet_success, rejected, run, workspace};
use std::fs;

#[test]
fn main_package_requires_provides_and_noarch_render_and_round_trip() {
    // Inject the three new main-package fields into the ed manifest. ed itself is
    // not noarch; this checks the rendering and round-trip, not an ed reproduction.
    let manifest = MANIFEST.replace(
        "[package.vcs]",
        "requires = [\"coreutils\", \"%{name}-libs = %{version}-%{release}\"]\n\
         provides = [\"ed-clone = %{version}\"]\n\
         noarch = true\n\n[package.vcs]",
    );
    let directory = workspace(&manifest);
    let output = run(directory.path(), &["gen", "ed", "--stdout"]);
    // gen succeeding means the render/verify round trip accepted the new tags.
    quiet_success(&output);
    let spec = String::from_utf8_lossy(&output.stdout);

    // BuildArch: noarch sits in the preamble block, immediately before BuildSystem.
    assert!(
        spec.contains("BuildArch:      noarch\nBuildSystem:    autotools\n"),
        "{spec}"
    );
    // Requires: block follows BuildRequires; Provides: follows Requires.
    let requires_at = spec.find("Requires:       coreutils").expect("requires");
    let buildrequires_at = spec.find("BuildRequires:").expect("buildrequires");
    let provides_at = spec.find("Provides:       ed-clone").expect("provides");
    assert!(buildrequires_at < requires_at, "{spec}");
    assert!(requires_at < provides_at, "{spec}");
    assert!(
        spec.contains("Requires:       %{name}-libs = %{version}-%{release}\n"),
        "{spec}"
    );
    // Both blocks precede %description.
    assert!(
        provides_at < spec.find("%description").expect("description"),
        "{spec}"
    );
}

#[test]
fn subpackages_publish_mixed_names_empty_files_and_build_stages() {
    let source = MANIFEST.replace(
        "[package.vcs]",
        "requires = ['main-runtime']\nprovides = ['main-api = 1']\n\n[package.vcs]",
    );
    let extra = r#"
[subpackages.meta]
summary = "Editor metapackage"
description = "Install the editor family."
requires = ["ed-devel", "editor-tools"]

[subpackages.editor-tools]
full-name = true
summary = "Editor tools"
description = "Standalone editor utilities."
requires = ["coreutils"]
provides = ["editor-helper = 1"]
[subpackages.editor-tools.files]
entries = ["%{_bindir}/editor-helper"]

[subpackages.devel]
summary = "Development files"
description = "Headers for ed development."
requires = ["%{name} = %{version}-%{release}", "pkgconfig(example) >= 1"]
provides = ["ed-devel-api = %{version}"]
[subpackages.devel.files]
license = ["COPYING.devel"]
doc = ["README.devel"]
entries = ["%{_includedir}/ed.h"]

[build.stages.prep]
prepend = "echo before prep"
[build.stages.build]
replace = "make all"
append = "echo after build"
[build.stages.check]
replace = ""
"#;
    let source = format!("{source}{extra}");
    let directory = workspace(&source);
    let stdout = run(directory.path(), &["gen", "ed", "--stdout"]);
    quiet_success(&stdout);
    assert!(!directory.path().join("ed.spec").exists());
    let generated = String::from_utf8(stdout.stdout.clone()).unwrap();
    let headers: Vec<_> = generated
        .lines()
        .filter(|line| line.starts_with("%package"))
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        headers,
        [
            "%package devel",
            "%package -n editor-tools",
            "%package meta"
        ]
    );
    for argument in ["devel", "-n editor-tools", "meta"] {
        for kind in ["%description", "%files"] {
            assert!(
                generated.lines().any(|line| {
                    line.split_whitespace().collect::<Vec<_>>().join(" ")
                        == format!("{kind} {argument}")
                }),
                "missing {kind} {argument}: {generated}"
            );
        }
    }
    for fact in [
        "Requires:       main-runtime\n",
        "Provides:       main-api = 1\n",
        "Summary:        Development files\n",
        "Requires:       %{name} = %{version}-%{release}\n",
        "Requires:       pkgconfig(example) >= 1\n",
        "Provides:       ed-devel-api = %{version}\n",
        "Headers for ed development.\n",
        "%files devel\n%license COPYING.devel\n%doc README.devel\n%{_includedir}/ed.h\n",
        "Summary:        Editor tools\n",
        "Requires:       coreutils\n",
        "Provides:       editor-helper = 1\n",
        "Standalone editor utilities.\n",
        "%files -n editor-tools\n%{_bindir}/editor-helper\n",
        "%prep -p\necho before prep\n",
        "%build\nmake all\n",
        "%build -a\necho after build\n",
        "%check\n\n",
    ] {
        assert!(generated.contains(fact), "{fact}: {generated}");
    }
    let nonempty_lines: Vec<_> = generated.lines().filter(|line| !line.is_empty()).collect();
    assert!(
        nonempty_lines
            .windows(2)
            .any(|lines| lines == ["%files meta", "%changelog"])
    );
    assert!(generated.find("%description    meta").unwrap() < generated.find("%prep -p").unwrap());
    assert!(generated.find("%check\n").unwrap() < generated.find("%files\n").unwrap());
    quiet_success(&run(directory.path(), &["gen", "ed"]));
    assert_eq!(
        fs::read(directory.path().join("ed.spec")).unwrap(),
        stdout.stdout
    );
    assert_file(directory.path().join("ed.toml"), &source);
}

#[test]
fn subpackage_names_and_effective_name_collisions_are_rejected_before_publication() {
    for (name, full_name) in [
        ("", false),
        ("two words", false),
        ("../devel", false),
        ("devel/tools", false),
        ("-devel", false),
        ("%{name}-devel", false),
        ("devel/tools", true),
    ] {
        let extra = format!(
            "\n[subpackages.\"{name}\"]\nfull-name = {full_name}\n\
             summary = 'Development files'\ndescription = 'Development package.'\n"
        );
        rejected(&format!("{MANIFEST}{extra}"), "subpackages");
    }
    for extra in [
        "\n[subpackages.ed]\nfull-name = true\nsummary = 'Main collision'\ndescription = 'Duplicate main.'\n",
        "\n[subpackages.devel]\nsummary = 'Development files'\ndescription = 'Development package.'\n\
         [subpackages.ed-devel]\nfull-name = true\nsummary = 'Same name'\ndescription = 'Duplicate subpackage.'\n",
    ] {
        rejected(&format!("{MANIFEST}{extra}"), "subpackages");
    }
}

#[test]
fn subpackage_body_rejects_invalid_and_unsupported_fields_before_publication() {
    let extra = "\n[subpackages.devel]\nsummary = 'Development files'\ndescription = 'Development package.'\n";
    for (before, after, field) in [
        ("summary = 'Development files'\n", "", "summary"),
        ("description = 'Development package.'\n", "", "description"),
        ("'Development files'", "''", "subpackages.devel.summary"),
        (
            "'Development package.'",
            "''",
            "subpackages.devel.description",
        ),
        (
            "'Development package.'",
            "\"Text\\n%files\\n/unexpected\"",
            "subpackages.devel.description",
        ),
    ] {
        rejected(
            &format!("{MANIFEST}{}", extra.replace(before, after)),
            field,
        );
    }
    for unsupported in [
        "noarch = true",
        "license = 'MIT'",
        "version = '2'",
        "build-requires = ['gcc']",
        "full_name = true",
        "files = { paths = ['/usr/include/ed.h'] }",
    ] {
        rejected(
            &format!("{MANIFEST}{extra}{unsupported}\n"),
            "unknown field",
        );
    }
    rejected(
        &format!("{MANIFEST}{extra}files = {{ entries = ['relative/path'] }}\n"),
        "subpackages.devel.files.entries",
    );
}

#[test]
fn malformed_subpackage_dependencies_cannot_overwrite_even_with_force() {
    for field in ["requires", "provides"] {
        let field_path = format!("subpackages.devel.{field}");
        for (value, diagnostic) in [
            ("example >=", "parser diagnostics"),
            ("example\\nSummary: injected", field_path.as_str()),
        ] {
            let source = format!(
                "{MANIFEST}\n[subpackages.devel]\nsummary = 'Development files'\n\
                 description = 'Development package.'\n{field} = [\"{value}\"]\n"
            );
            // Both a malformed dependency expression and a tag-injection attempt
            // must fail before any destination is created or replaced.
            rejected(&source, diagnostic);
            let directory = workspace(&source);
            let destination = directory.path().join("ed.spec");
            fs::write(&destination, "# maintained by hand\n").unwrap();
            let output = run(directory.path(), &["gen", "ed", "--force"]);
            assert_eq!(output.status.code(), Some(1), "{field}: {output:?}");
            assert!(output.stdout.is_empty());
            assert!(!output.stderr.is_empty());
            assert_file(&destination, "# maintained by hand\n");
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
        }
    }
}
