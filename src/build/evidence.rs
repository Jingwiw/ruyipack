// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Compare a delivery's inputs with a retained execution, without running a build.

use super::{Engine, InputFile, Stage, invalid};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

// A remote declaration fixes bytes, but does not promise a filesystem mode.
pub(crate) type Inputs = BTreeMap<String, (String, Option<bool>)>;

#[derive(Serialize)]
pub(crate) struct Evidence {
    pub(crate) status: &'static str,
    receipt: PathBuf,
    stage: Option<Stage>,
    success: Option<bool>,
    image: Option<String>,
    architecture: Option<String>,
    release_policy: Option<String>,
    pub(crate) differences: Vec<String>,
    pub(crate) reason: Option<String>,
}

#[derive(Deserialize)]
struct Receipt {
    format_version: u32,
    package: String,
    engine: String,
    stage: Stage,
    success: bool,
    inputs: Vec<InputFile>,
    execution: serde_json::Value,
}

pub(crate) fn compare(directory: &Path, package: &str, expected: &Inputs) -> Evidence {
    let mut result = Evidence {
        status: "not-run",
        receipt: directory.join("receipt.json"),
        stage: None,
        success: None,
        image: None,
        architecture: None,
        release_policy: None,
        differences: vec![],
        reason: None,
    };
    match fs_err::symlink_metadata(&result.receipt) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return result,
        _ => {}
    }
    if let Err(error) = inspect(directory, package, expected, &mut result) {
        result.status = "unavailable";
        result.reason = Some(error.to_string());
    }
    result
}

fn inspect(
    directory: &Path,
    package: &str,
    expected: &Inputs,
    result: &mut Evidence,
) -> io::Result<()> {
    super::regular_file(&result.receipt)?;
    let receipt: Receipt = serde_json::from_slice(&fs_err::read(&result.receipt)?)
        .map_err(|e| invalid(e.to_string()))?;
    if receipt.format_version != 1 || receipt.package != package || receipt.engine != "mock" {
        return Err(invalid(
            "build receipt does not identify this package and engine",
        ));
    }
    result.stage = Some(receipt.stage);
    result.success = Some(receipt.success);
    result.image = receipt.execution["details"]["image_id"]
        .as_str()
        .map(str::to_owned);
    let engine_path = directory.join("engine/receipt.json");
    let engine = if engine_path.try_exists()? {
        super::regular_file(&engine_path)?;
        serde_json::from_slice::<serde_json::Value>(&fs_err::read(engine_path)?)
            .map_err(|e| invalid(e.to_string()))?
    } else {
        serde_json::Value::Null
    };
    result.architecture = engine["target"]["architecture"].as_str().map(str::to_owned);
    result.release_policy = engine["target"]["release_policy"]
        .as_str()
        .map(str::to_owned);
    if !receipt.success {
        result.reason = receipt.execution["failure"].as_str().map(str::to_owned);
    }
    let mut recorded = BTreeMap::new();
    for input in receipt.inputs {
        if recorded
            .insert(input.path, (input.content, input.executable))
            .is_some()
        {
            return Err(invalid("duplicate build input identity"));
        }
    }
    for (name, (hash, executable)) in expected {
        let Some((content, built_executable)) = recorded.get(name) else {
            result.differences.push(name.clone());
            continue;
        };
        let path = directory.join("input").join(name);
        if crate::file_digest::read(&path).map_err(io::Error::other)? != *content {
            return Err(invalid(format!("{name}: retained build input changed")));
        }
        if crate::file_digest::executable(&fs_err::symlink_metadata(&path)?) != *built_executable {
            return Err(invalid(format!(
                "{name}: retained build input mode changed"
            )));
        }
        let mode_matches = executable.is_none_or(|mode| mode == *built_executable);
        if !hash.eq_ignore_ascii_case(&content.sha256) || !mode_matches {
            result.differences.push(name.clone());
        }
    }
    if receipt.success && result.differences.is_empty() {
        verify_environment(directory, &receipt.execution, &engine, result)?;
        super::mock::Mock(receipt.stage).verify_result(&directory.join("engine"))?;
    }
    result.status = if !result.differences.is_empty() {
        "stale"
    } else if receipt.success {
        "passed"
    } else {
        "failed"
    };
    Ok(())
}

/// Require retained environment facts before a successful process becomes build evidence.
fn verify_environment(
    directory: &Path,
    execution: &serde_json::Value,
    engine: &serde_json::Value,
    result: &Evidence,
) -> io::Result<()> {
    if result.image.as_deref().is_none_or(str::is_empty)
        || result.architecture.as_deref().is_none_or(str::is_empty)
        || result.release_policy.as_deref().is_none_or(str::is_empty)
    {
        return Err(invalid("build environment identity is incomplete"));
    }
    let packages = engine["installed_packages"]
        .as_array()
        .filter(|packages| !packages.is_empty())
        .ok_or_else(|| invalid("installed build dependencies are not recorded"))?;
    if packages.iter().any(|package| {
        ["name", "evr", "arch"]
            .iter()
            .any(|key| package[key].as_str().is_none_or(str::is_empty))
    }) || engine["collection_errors"]
        .as_array()
        .is_none_or(|errors| !errors.is_empty())
    {
        return Err(invalid("build environment collection is incomplete"));
    }
    let config = directory.join("engine/mock.cfg");
    super::regular_file(&config)?;
    if engine["mock_config_sha256"].as_str()
        != Some(
            crate::file_digest::read(&config)
                .map_err(io::Error::other)?
                .sha256
                .as_str(),
        )
    {
        return Err(invalid("retained Mock configuration changed"));
    }
    let target = directory.join("engine/target.json");
    super::regular_file(&target)?;
    let target: serde_json::Value =
        serde_json::from_slice(&fs_err::read(target)?).map_err(io::Error::other)?;
    if target != engine["target"] || execution["success"] != true {
        return Err(invalid("engine and host build evidence disagree"));
    }
    Ok(())
}

impl Evidence {
    pub(crate) fn full_build_passed(&self) -> bool {
        self.status == "passed" && matches!(self.stage, Some(Stage::Build))
    }

    pub(crate) fn stage_name(&self) -> &'static str {
        match self.stage {
            Some(Stage::Build) => "build",
            Some(Stage::Prep) => "prep",
            None => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn evidence_compares_bytes_and_scope_without_promoting_prep_to_build() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input/SPECS/ed.spec");
        fs::create_dir_all(input.parent().unwrap()).unwrap();
        fs::write(&input, "Name: ed\n").unwrap();
        let content = crate::file_digest::read(&input).unwrap();
        let mut expected = Inputs::from([("SPECS/ed.spec".into(), (content.sha256.clone(), None))]);
        assert_eq!(compare(root.path(), "ed", &expected).status, "not-run");
        let mut receipt = serde_json::json!({"format_version":1,"package":"ed","engine":"mock","stage":"build","success":true,"execution":{"success":true,"details":{"image_id":"sha256:fixture"}},"inputs":[{"path":"SPECS/ed.spec","executable":false,"size":content.size,"sha256":content.sha256}]});
        let path = root.path().join("receipt.json");
        let save = |value: &serde_json::Value| {
            fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        };
        save(&receipt);
        assert_eq!(compare(root.path(), "ed", &expected).status, "unavailable");
        let engine = root.path().join("engine");
        fs::create_dir(&engine).unwrap();
        fs::write(engine.join("mock.cfg"), "fixture").unwrap();
        fs::write(engine.join("artifact.rpm"), "fixture").unwrap();
        let target = serde_json::json!({"architecture":"x86_64","release_policy":"fixture"});
        fs::write(engine.join("target.json"), target.to_string()).unwrap();
        let digest = crate::utf8_file::sha256("fixture");
        let mut native = serde_json::json!({
            "format_version":1,"engine":"mock","target_stage":"build","success":true,
            "target":target,"mock_config_sha256":digest,
            "installed_packages":[{"name":"rpm","evr":"6","arch":"x86_64"}],
            "collection_errors":[],
            "artifacts":[{"path":"artifact.rpm","size":7,"sha256":digest}]
        });
        let save_native = |value: &serde_json::Value| {
            fs::write(engine.join("receipt.json"), value.to_string()).unwrap();
        };
        save_native(&native);
        assert_eq!(compare(root.path(), "ed", &expected).status, "passed");
        for field in [
            "installed_packages",
            "collection_errors",
            "mock_config_sha256",
            "target",
        ] {
            let mut damaged = native.clone();
            damaged.as_object_mut().unwrap().remove(field);
            save_native(&damaged);
            assert_eq!(
                compare(root.path(), "ed", &expected).status,
                "unavailable",
                "{field}"
            );
        }
        save_native(&native);
        for name in ["mock.cfg", "artifact.rpm", "target.json"] {
            let path = engine.join(name);
            let original = fs::read(&path).unwrap();
            fs::write(&path, "changed").unwrap();
            assert_eq!(
                compare(root.path(), "ed", &expected).status,
                "unavailable",
                "{name}"
            );
            fs::write(path, original).unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            expected.get_mut("SPECS/ed.spec").unwrap().1 = Some(true);
            assert_eq!(compare(root.path(), "ed", &expected).status, "stale");
            fs::set_permissions(&input, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(compare(root.path(), "ed", &expected).status, "unavailable");
            fs::set_permissions(&input, fs::Permissions::from_mode(0o644)).unwrap();
            expected.get_mut("SPECS/ed.spec").unwrap().1 = None;
        }
        native["target_stage"] = "prep".into();
        save_native(&native);
        receipt["stage"] = "prep".into();
        save(&receipt);
        let prep = compare(root.path(), "ed", &expected);
        assert_eq!(prep.status, "passed");
        assert_eq!(prep.stage_name(), "prep");
        assert!(!prep.full_build_passed());
        native["target_stage"] = "build".into();
        save_native(&native);
        receipt["stage"] = "build".into();
        save(&receipt);
        assert!(compare(root.path(), "ed", &expected).full_build_passed());

        receipt["success"] = false.into();
        save(&receipt);
        assert_eq!(compare(root.path(), "ed", &expected).status, "failed");
        expected.get_mut("SPECS/ed.spec").unwrap().0 = "0".repeat(64);
        assert_eq!(compare(root.path(), "ed", &expected).status, "stale");
        fs::write(&input, "tampered snapshot").unwrap();
        assert_eq!(compare(root.path(), "ed", &expected).status, "unavailable");
        fs::write(&path, "invalid receipt").unwrap();
        assert_eq!(compare(root.path(), "ed", &expected).status, "unavailable");
    }
}
