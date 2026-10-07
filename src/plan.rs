// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Package selection and operation settings shared by remote submission and PRs.

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Settings {
    pub project: Option<String>,
    pub parent: Option<String>,
    pub repositories: Option<Vec<String>>,
    pub publish: Option<bool>,
    pub service: Option<String>,
    pub constraints: Option<String>,
}
impl Settings {
    pub fn inherit(&self, parent: &Self) -> Self {
        Self {
            project: self
                .project
                .clone()
                .filter(|x| x != "default")
                .or_else(|| parent.project.clone()),
            parent: self
                .parent
                .clone()
                .filter(|x| x != "default")
                .or_else(|| parent.parent.clone()),
            repositories: self
                .repositories
                .clone()
                .or_else(|| parent.repositories.clone()),
            publish: self.publish.or(parent.publish),
            service: self.service.clone().or_else(|| parent.service.clone()),
            constraints: self
                .constraints
                .clone()
                .or_else(|| parent.constraints.clone()),
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    #[serde(flatten)]
    pub defaults: Settings,
    pub packages: Vec<Task>,
    pub pr: Option<Publication>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    pub work: String,
    #[serde(flatten)]
    pub settings: Settings,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Publication {
    pub title: String,
    pub base: String,
    pub target: Option<String>,
    pub template: Option<std::path::PathBuf>,
}

/// Validate the whole batch before any consumer prepares WORKs or contacts a service.
pub(crate) fn validate_works<'a>(works: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for work in works {
        crate::check::metadata::Field::Name.validate_at(work, "WORK")?;
        if !seen.insert(work) {
            return Err(format!("duplicate WORK: {work}"));
        }
    }
    if seen.is_empty() {
        return Err("plan contains no packages".into());
    }
    Ok(())
}
