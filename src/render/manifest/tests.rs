// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

use super::*;

const INPUT: &str = include_str!("../../../examples/ed/ed.toml");

#[test]
fn snapshot_uses_current_values_and_preserves_material_order_and_empty_replacement() {
    let input = INPUT.replace("no-public-repository = true", "")
        + r#"
[sources.12]
path = "last-source"
[sources.2]
path = "second-source"
[patches.9]
path = "first.patch"
[patches.1]
path = "second.patch"
[build.stages.conf]
replace = ""
[build.stages.build]
"#;
    let mut manifest = parse_document(toml::from_str(&input).unwrap()).unwrap();
    let Source::Remote { sha256, .. } = &mut manifest.sources[0].1 else {
        panic!("fixture Source0 must be remote");
    };
    *sha256 = Some("a".repeat(64));

    #[derive(Serialize)]
    struct Snapshot<'a> {
        manifest: &'a Manifest,
        profile: &'a crate::profile::Profile,
    }
    let output = toml::to_string_pretty(&Snapshot {
        manifest: &manifest,
        profile: crate::profile::load(),
    })
    .unwrap();
    let output: toml::Table = toml::from_str(&output).unwrap();
    let snapshot = &output["manifest"];
    let sources = snapshot["sources"].as_array().unwrap();
    assert_eq!(
        sources
            .iter()
            .map(|source| source["number"].as_integer().unwrap())
            .collect::<Vec<_>>(),
        [0, 2, 12]
    );
    assert_eq!(sources[0]["kind"].as_str(), Some("remote"));
    assert_eq!(sources[0]["sha256"].as_str(), Some("a".repeat(64).as_str()));
    assert_eq!(sources[1]["kind"].as_str(), Some("local"));
    assert_eq!(sources[1]["path"].as_str(), Some("second-source"));
    assert_eq!(
        snapshot["patches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|patch| patch["number"].as_integer().unwrap())
            .collect::<Vec<_>>(),
        [9, 1]
    );
    assert_eq!(snapshot["package"]["vcs"]["kind"].as_str(), Some("unknown"));
    assert_eq!(
        snapshot["build"]["stages"]["conf"]["replace"].as_str(),
        Some("")
    );
    assert!(
        snapshot["build"]["stages"]["build"]
            .get("replace")
            .is_none()
    );
    assert_eq!(output["profile"]["release"].as_str(), Some("%autorelease"));
    assert_eq!(
        output["profile"]["changelog"].as_str(),
        Some("%autochangelog")
    );
}

#[test]
fn snapshot_discriminates_vcs_and_subpackage_name_forms() {
    for (vcs, kind, url) in [
        (Vcs::Unknown, "unknown", None),
        (
            Vcs::Git("https://example.org/repo".into()),
            "git",
            Some("https://example.org/repo"),
        ),
        (Vcs::SameAsUrl, "same-as-url", None),
        (Vcs::NoPublicRepository, "no-public-repository", None),
    ] {
        let value = toml::Value::try_from(&vcs).unwrap();
        assert_eq!(value["kind"].as_str(), Some(kind));
        assert_eq!(value.get("url").and_then(toml::Value::as_str), url);
    }
    for (name, kind) in [
        (SubpackageName::Suffix("devel".into()), "suffix"),
        (SubpackageName::Absolute("devel".into()), "absolute"),
    ] {
        let value = toml::Value::try_from(&name).unwrap();
        assert_eq!(value["kind"].as_str(), Some(kind));
        assert_eq!(value["value"].as_str(), Some("devel"));
    }
}

#[test]
fn omitted_build_requirement_list_selects_the_declared_contract() {
    let mut input: toml::Table = toml::from_str(INPUT).unwrap();
    for system in crate::profile::buildsystems::systems() {
        input["build"]["system"] = system.into();
        for table in [None, Some(toml::Table::new())] {
            input.remove("build-requires");
            if let Some(table) = table {
                input.insert("build-requires".into(), toml::Value::Table(table));
            }
            let manifest = parse_document(input.clone()).unwrap();
            assert_eq!(
                manifest.build_requires.rpm,
                crate::profile::buildsystems::contract(system)
                    .unwrap()
                    .build_requires
            );
        }
    }
    input.insert(
        "build-requires".into(),
        toml::Value::Table(toml::from_str("rpm = []").unwrap()),
    );
    assert!(
        parse_document(input.clone())
            .unwrap()
            .build_requires
            .rpm
            .is_empty()
    );
    input.remove("build-requires");
    input.remove("build");
    assert!(
        parse_document(input.clone())
            .unwrap()
            .build_requires
            .rpm
            .is_empty()
    );

    input["package"].as_table_mut().unwrap().remove("files");
    let error = parse_document(input).err().unwrap();
    assert!(
        error
            .to_string()
            .contains("package.files: at least one file entry is required")
    );
}

#[test]
fn snapshot_material_tags_are_not_authoring_input() {
    let tagged = INPUT.replace("[sources.0]", "[sources.0]\nkind = \"remote\"");
    assert!(parse(&tagged).is_err());
}
