// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

use super::{Report, baseline, invalid};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};

pub(super) fn line(value: &str) -> io::Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(invalid(
            "title and summary entries must be nonempty single lines",
        ));
    }
    Ok(())
}

pub(super) fn load(
    explicit: Option<&Path>,
    workspace: &Path,
    overrides: impl IntoIterator<Item = PathBuf>,
) -> io::Result<String> {
    if let Some(path) = explicit {
        return crate::utf8_file::read(path).map_err(io::Error::other);
    }
    let mut selected = None;
    for path in overrides {
        let path = if path.try_exists()? {
            path.as_path()
        } else {
            workspace
        };
        let text = crate::utf8_file::read(path).map_err(io::Error::other)?;
        if selected.as_ref().is_some_and(|previous| previous != &text) {
            return Err(invalid(
                "selected WORKs use different PR templates; select one with --template",
            ));
        }
        selected = Some(text);
    }
    selected.ok_or_else(|| invalid("PR plan selects no WORKs"))
}

pub(super) fn render(
    template: &str,
    summary: &str,
    links: &BTreeSet<String>,
) -> io::Result<String> {
    if summary.is_empty() {
        return Err(invalid("PR summary is empty"));
    }
    let links = links
        .iter()
        .map(|url| format!("[OBS build project]({url})"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut output = String::new();
    let mut rest = template;
    let mut seen = BTreeSet::new();
    while let Some((prefix, after)) = rest.split_once("{{") {
        output.push_str(prefix);
        let (key, after) = after
            .split_once("}}")
            .ok_or_else(|| invalid("unclosed PR template placeholder"))?;
        let value = match key {
            "summary" => summary,
            "obs_links" => &links,
            _ => return Err(invalid(format!("unknown PR template placeholder: {key}"))),
        };
        if !seen.insert(key) {
            return Err(invalid(format!("repeated PR template placeholder: {key}")));
        }
        output.push_str(value);
        rest = after;
    }
    output.push_str(rest);
    if seen != BTreeSet::from(["summary", "obs_links"]) {
        return Err(invalid(
            "PR template requires {{summary}} and {{obs_links}}",
        ));
    }
    Ok(output)
}

// Read only the receipt fields used for identity and links. OBS owns the rest.
#[derive(Deserialize)]
struct Receipt {
    api: String,
    project: String,
    package: String,
    sha256: BTreeMap<String, String>,
}

pub(super) fn obs_link(
    area: &crate::workspace::Development,
    files: &baseline::Files,
    report: &mut Report,
) -> io::Result<()> {
    let path = area.directory().join("remote-receipt.toml");
    if !path.try_exists()? {
        report
            .warnings
            .push(format!("{}: no remote-build receipt", area.package()));
        return Ok(());
    }
    let receipt: Receipt = baseline::load(&path)?;
    if receipt.package != area.package()
        || files
            .iter()
            .any(|(name, file)| receipt.sha256.get(name) != Some(&file.sha256))
    {
        report.warnings.push(format!(
            "{}: remote-build receipt does not match committed files; link omitted",
            area.package()
        ));
        return Ok(());
    }
    let mut url = url::Url::parse(&receipt.api).map_err(io::Error::other)?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid("invalid OBS receipt API URL"));
    }
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut()
        .map_err(|()| invalid("OBS API has no hierarchical path"))?
        .clear()
        .extend(["project", "show", &receipt.project]);
    report.obs_links.insert(url.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_defaults_and_work_overrides_require_one_body() {
        let root = tempfile::tempdir().unwrap();
        let default = root.path().join("default.md");
        let first = root.path().join("first.md");
        let second = root.path().join("second.md");
        std::fs::write(&default, "default").unwrap();
        assert_eq!(
            load(None, &default, [first.clone(), second.clone()]).unwrap(),
            "default"
        );
        std::fs::write(&first, "custom").unwrap();
        assert_eq!(load(None, &default, [first.clone()]).unwrap(), "custom");
        assert!(load(None, &default, [first.clone(), second.clone()]).is_err());
        std::fs::write(&second, "custom").unwrap();
        assert_eq!(
            load(None, &default, [first.clone(), second.clone()]).unwrap(),
            "custom"
        );
        assert_eq!(
            load(Some(&default), &default, [first, second]).unwrap(),
            "default"
        );
    }

    #[test]
    fn template_inserts_data_once_and_rejects_unknown_slots() {
        let summary = "- Keep {{obs_links}} as data.";
        let links = BTreeSet::from(["https://obs.example/project/show/home:alice:test".into()]);
        assert_eq!(
            render("{{summary}}\n{{obs_links}}", summary, &links).unwrap(),
            "- Keep {{obs_links}} as data.\n[OBS build project](https://obs.example/project/show/home:alice:test)"
        );
        assert!(render("{{summary}} {{status}}", summary, &links).is_err());
        assert!(render("{{summary}}", summary, &links).is_err());
    }
}
