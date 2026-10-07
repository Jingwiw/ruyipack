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

/// Resolve plan-owned paths and reject duplicate WORKs before any action runs.
pub(crate) fn load(paths: &[std::path::PathBuf]) -> std::io::Result<Vec<Plan>> {
    let plans = paths
        .iter()
        .map(|path| {
            let mut plan: Plan = crate::workspace::baseline::load(path)?;
            if let Some(template) = plan.pr.as_mut().and_then(|p| p.template.as_mut()) {
                *template = std::fs::canonicalize(path)?
                    .parent()
                    .expect("plan parent")
                    .join(&*template);
            }
            Ok(plan)
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    validate_works(
        plans
            .iter()
            .flat_map(|p| p.packages.iter().map(|t| t.work.as_str())),
    )
    .map_err(std::io::Error::other)?;
    Ok(plans)
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
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn plan_overrides_inherit_without_copying_credentials() {
        let plan: super::Plan = toml::from_str(
            "project = 'home:alice:test'\nparent = 'default'\n[[packages]]\nwork = 'ed'\n",
        )
        .unwrap();
        let global = super::Settings {
            parent: Some("openruyi".into()),
            repositories: Some(vec!["x86_64".into()]),
            publish: Some(false),
            ..Default::default()
        };
        let settings = plan.packages[0]
            .settings
            .inherit(&plan.defaults.inherit(&global));
        assert_eq!(settings.project.as_deref(), Some("home:alice:test"));
        assert_eq!(settings.parent.as_deref(), Some("openruyi"));
        assert_eq!(settings.publish, Some(false));
        assert!(toml::from_str::<super::Plan>("password='secret'\npackages=[]").is_err());
    }
}
