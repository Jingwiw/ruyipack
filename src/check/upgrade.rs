// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Repology observations are version candidates, not proof of build compatibility.

use crate::spec::{ParsedSpec, document::Snapshot};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};

#[derive(Serialize)]
pub(crate) struct Report {
    pub(crate) project: Option<String>,
    pub(crate) current: Option<String>,
    pub(crate) candidate: Option<String>,
    pub(crate) status: &'static str,
    pub(crate) error: Option<String>,
    pub(crate) input_sha256: String,
}

impl Report {
    pub(crate) fn applies_candidate(&self) -> bool {
        self.error.is_none() && self.candidate.is_some()
    }
}

#[derive(Deserialize)]
struct Package {
    version: String,
    status: String,
}

pub(crate) fn query(source: &str, override_project: Option<&str>) -> Report {
    query_candidate(source, override_project).unwrap_or_else(|error| Report {
        project: override_project.map(str::to_owned),
        current: None,
        candidate: None,
        status: "unavailable",
        error: Some(error),
        input_sha256: crate::utf8_file::sha256(source),
    })
}

fn query_candidate(source: &str, override_project: Option<&str>) -> Result<Report, String> {
    let parsed = ParsedSpec::parse(source);
    let snapshot =
        Snapshot::capture_selected(&parsed, &["package.name".into(), "package.version".into()]);
    let literal = snapshot.as_ref().ok().and_then(|snapshot| {
        let package = &snapshot.document()["package"];
        let name = package["name"].as_str()?;
        let version = package["version"].as_str()?;
        (!name.contains('%') && !version.contains('%')).then(|| {
            std::collections::BTreeMap::from([
                ("name", name.to_owned()),
                ("version", version.to_owned()),
            ])
        })
    });
    let identity = literal
        .map(Ok)
        .unwrap_or_else(|| crate::spec::identity::declarations(&parsed))?;
    let name = identity["name"].as_str();
    let current = identity["version"].as_str();
    let workspace = crate::workspace::discover_optional().map_err(|e| e.to_string())?;
    let override_project =
        override_project.or_else(|| workspace.as_ref().and_then(|w| w.repology_project(name)));
    let project = override_project
        .map(str::to_owned)
        .unwrap_or_else(|| project_name(name));
    let mut report = Report {
        project: Some(project.clone()),
        current: Some(current.into()),
        candidate: None,
        status: "unavailable",
        error: None,
        input_sha256: crate::utf8_file::sha256(source),
    };
    match fetch(&project).and_then(|rows| select(&rows, current)) {
        Ok(candidate) => {
            report.status = if candidate.is_some() {
                "upgrade-available"
            } else {
                "current"
            };
            report.candidate = candidate;
            if report.candidate.is_some()
                && let Err(reason) = automatic_upgrade(&parsed)
            {
                report.status = "review-required";
                report.error = Some(reason);
            }
        }
        Err(error) => report.error = Some(error),
    }
    Ok(report)
}

fn automatic_upgrade(parsed: &ParsedSpec<'_>) -> Result<(), String> {
    // Query identity does not grant permission to replace an author's macro.
    let snapshot = Snapshot::capture_selected(parsed, &["package.version".into()])?;
    if snapshot.document()["package"]["version"]
        .as_str()
        .is_none_or(|v| v.contains('%'))
    {
        return Err("macro Version requires manual review; upgrade not applied".into());
    }
    let materials = crate::spec::sources::resolve_materials(parsed, &[]).map_err(|reason| {
        format!("material declarations require review; upgrade not applied: {reason}")
    })?;
    if !materials.patches.is_empty() {
        return Err("Patch declarations require manual review; upgrade not applied".into());
    }
    if let Some(reason) = materials.incomplete {
        return Err(format!(
            "material declarations are incomplete; upgrade not applied: {reason}"
        ));
    }
    Ok(())
}

fn project_name(name: &str) -> String {
    if let Some(name) = name
        .strip_prefix("python3-")
        .or_else(|| name.strip_prefix("python-"))
    {
        format!("python:{}", name.to_ascii_lowercase().replace('_', "-"))
    } else {
        name.to_ascii_lowercase()
    }
}

fn fetch(project: &str) -> Result<Vec<Package>, String> {
    let mut url = url::Url::parse("https://repology.org/api/v1/project/").expect("constant URL");
    url.path_segments_mut()
        .expect("HTTP URL")
        .pop_if_empty()
        .push(project);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .build()
        .into();
    let body = agent
        .get(url.as_str())
        .header(
            "User-Agent",
            "ruyipack/0.0.1 (https://github.com/Jingwiw/ruyipack)",
        )
        .call()
        .map_err(|e| format!("Repology {project}: {e}"))?
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("Repology response: {e}"))?;
    serde_json::from_slice(&body).map_err(|e| format!("Repology response: {e}"))
}

fn select(rows: &[Package], current: &str) -> Result<Option<String>, String> {
    let newest: BTreeSet<_> = rows
        .iter()
        .filter(|p| p.status == "newest")
        .map(|p| p.version.as_str())
        .collect();
    let latest = newest
        .into_iter()
        .max_by(|a, b| rpmvercmp_rs::rpmvercmp(a.as_bytes(), b.as_bytes()))
        .ok_or("Repology has no newest version; review the project mapping")?;
    if latest == current {
        return Ok(None);
    }
    if rpmvercmp_rs::rpmvercmp(latest.as_bytes(), current.as_bytes()) != std::cmp::Ordering::Greater
    {
        return Err(
            "Repology candidate is not newer under RPM version ordering; refusing downgrade".into(),
        );
    }
    if !rows
        .iter()
        .any(|p| p.version == current && p.status == "outdated")
    {
        return Err("current version is not classified as outdated; review the project mapping and version scheme".into());
    }
    if latest.is_empty()
        || !latest
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+~-".contains(&c))
    {
        return Err("Repology version cannot be used as a literal RPM Version".into());
    }
    Ok(Some(latest.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_upgrade_requires_complete_patch_free_materials() {
        let plain = "Name: sample\nVersion: 1\n";
        assert!(automatic_upgrade(&ParsedSpec::parse(plain)).is_ok());
        let patched = format!("{plain}Patch0: fix.patch\n");
        assert!(
            automatic_upgrade(&ParsedSpec::parse(&patched))
                .unwrap_err()
                .contains("Patch")
        );
        let macro_version = "%global v 1\nName: sample\nVersion: %{v}\n";
        assert!(
            automatic_upgrade(&ParsedSpec::parse(macro_version))
                .unwrap_err()
                .contains("macro Version")
        );
        let included = format!("{plain}%include other.spec\n");
        assert!(automatic_upgrade(&ParsedSpec::parse(&included)).is_err());
    }

    #[test]
    fn versions_require_provider_evidence_not_lexical_order() {
        let rows: Vec<Package> = serde_json::from_str(
            r#"[{"version":"9.0","status":"outdated"},{"version":"10.0","status":"newest"}]"#,
        )
        .unwrap();
        assert_eq!(select(&rows, "9.0").unwrap().as_deref(), Some("10.0"));
        assert_eq!(select(&rows, "10.0").unwrap(), None);
        assert!(select(&rows, "11.0").is_err());
        assert!(select(&[], "9.0").is_err());
        let ranked: Vec<Package> = serde_json::from_str(r#"[{"version":"9.0","status":"outdated"},{"version":"10.0","status":"newest"},{"version":"11.0","status":"newest"},{"version":"12.0","status":"devel"}]"#).unwrap();
        assert_eq!(select(&ranked, "9.0").unwrap().as_deref(), Some("11.0"));

        let downgrade: Vec<Package> = serde_json::from_str(
            r#"[{"version":"1.37.0","status":"outdated"},{"version":"1.36.1","status":"newest"}]"#,
        )
        .unwrap();
        assert!(
            select(&downgrade, "1.37.0")
                .unwrap_err()
                .contains("downgrade")
        );

        assert_eq!(
            project_name("python3-typing_extensions"),
            "python:typing-extensions"
        );
    }
}
