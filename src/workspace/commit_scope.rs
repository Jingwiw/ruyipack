// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

use super::{Workspace, baseline, invalid};
use serde::Deserialize;
use std::{collections::BTreeSet, io};

pub(super) const DEFAULT: &str = "files = [\"_constraints\"]\n";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Exclusions {
    files: BTreeSet<String>,
}
impl Exclusions {
    pub fn load(workspace: &Workspace) -> io::Result<Self> {
        let path = workspace.configuration().join("commit-ignore.toml");
        let rules: Self = if path.try_exists()? {
            baseline::load(&path)?
        } else {
            toml::from_str(DEFAULT)
                .map_err(|e| invalid(format!("invalid commit exclusions: {e}")))?
        };
        for name in &rules.files {
            baseline::relative(name)?;
        }
        Ok(rules)
    }

    pub fn filter(&self, files: &mut baseline::Files) {
        files.retain(|name, _| !self.files.contains(name));
    }

    pub fn check(&self, files: &baseline::Files) -> io::Result<()> {
        if let Some(name) = files.keys().find(|name| self.files.contains(*name)) {
            return Err(invalid(format!(
                "{name}: excluded from Git publication but present in the repository or pending commit; remove it in a separate reviewed commit"
            )));
        }
        Ok(())
    }
}
