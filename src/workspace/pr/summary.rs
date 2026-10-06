// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

use super::{invalid, template::line};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Default)]
pub(super) struct Summary {
    actions: BTreeMap<String, BTreeSet<String>>,
    notes: BTreeMap<String, Vec<String>>,
}

impl Summary {
    pub fn add(&mut self, package: &str, message: &str) -> io::Result<()> {
        let (subject, trailers) = message.split_once('\n').unwrap_or((message, ""));
        let title = subject
            .strip_prefix(&format!("SPECS: {package}: "))
            .ok_or_else(|| {
                invalid(format!(
                    "{package}: commit subject must start with SPECS: {package}: "
                ))
            })?;
        // Action trailers preserve boundaries even when an action contains commas
        // or 'and'. Older mechanical subjects use comma lists and a final 'and'.
        let actions: Vec<_> = if trailers.trim().is_empty() {
            title
                .split(", ")
                .flat_map(|part| part.split(" and "))
                .collect()
        } else {
            trailers.lines().filter(|line| !line.is_empty()).collect()
        };
        for action in actions {
            line(action)?;
            let mut action = action.to_owned();
            // Sentence capitalization is presentation, not another action.
            if let Some(first) = action.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            self.actions
                .entry(action)
                .or_default()
                .insert(package.to_owned());
        }
        Ok(())
    }

    pub fn note(&mut self, package: &str, text: &str) {
        let notes = self.notes.entry(package.to_owned()).or_default();
        if !notes.iter().any(|note| note == text) {
            notes.push(text.to_owned());
        }
    }

    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        let mut unique = BTreeMap::<&str, Vec<&str>>::new();
        for (action, packages) in &self.actions {
            if packages.len() > 1 {
                lines.push(format!("- {action}"));
            } else if let Some(package) = packages.first() {
                unique.entry(package).or_default().push(action);
            }
        }
        for package in self.notes.keys() {
            unique.entry(package).or_default();
        }
        for (package, actions) in unique {
            // A package with only shared actions still needs an anchor for its note.
            lines.push(
                format!("- {package}: {}", actions.join("; "))
                    .trim_end()
                    .to_owned(),
            );
            if let Some(notes) = self.notes.get(package) {
                lines.extend(notes.iter().map(|note| format!("  - {note}")));
            }
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_actions_by_distinct_packages_and_attaches_notes_once() {
        let mut summary = Summary::default();
        summary
            .add("a", "SPECS: a: Add hashes, use changelog and fix tests")
            .unwrap();
        summary
            .add("b", "SPECS: b: Add hashes and use changelog")
            .unwrap();
        summary.add("a", "SPECS: a: Fix tests").unwrap();
        summary.note("a", "Tests need memory.");
        summary.note("b", "Tests need permissions.");
        summary.note("b", "Tests need permissions.");
        assert_eq!(
            summary.render(),
            "- Add hashes\n- Use changelog\n- a: Fix tests\n  - Tests need memory.\n- b:\n  - Tests need permissions."
        );
    }
    #[test]
    fn explicit_actions_preserve_punctuation_and_conjunctions() {
        let mut summary = Summary::default();
        summary.add("a", "SPECS: a: Preserve read and write, not only read\nPreserve read and write, not only read\n").unwrap();
        assert_eq!(
            summary.render(),
            "- a: Preserve read and write, not only read"
        );
        assert!(summary.add("b", "SPECS: a: Wrong package").is_err());
    }
}
