// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

use super::{
    api::{Client, parse, xml},
    config::Settings,
};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::{collections::BTreeMap, io::Read, path::Path};

#[derive(Serialize, Deserialize)]
pub(super) struct Receipt {
    pub api: String,
    pub project: String,
    pub package: String,
    pub revision: String,
    pub files: BTreeMap<String, String>,
    pub sha256: BTreeMap<String, String>,
}
pub(super) struct Input {
    pub directory: tempfile::TempDir,
    pub files: BTreeMap<String, String>,
    pub sha256: BTreeMap<String, String>,
}
fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    let before = crate::file_digest::read(source).map_err(|e| e.to_string())?;
    if target.exists() {
        if before != crate::file_digest::read(target).map_err(|e| e.to_string())? {
            return Err(format!("conflicting upload filename: {}", target.display()));
        }
    } else {
        fs_err::copy(source, target).map_err(|e| e.to_string())?;
    }
    if before != crate::file_digest::read(target).map_err(|e| e.to_string())? {
        return Err("material changed while staging".into());
    }
    Ok(())
}
pub(super) fn stage(
    area: &crate::workspace::Development,
    service: &str,
    constraints: Option<&str>,
) -> Result<Input, String> {
    let (spec, source, _) = area.source().map_err(|e| e.to_string())?;
    let parsed = crate::spec::ParsedSpec::parse(&source);
    if parsed
        .diagnostics()
        .iter()
        .any(|d| matches!(d.severity, crate::parser_diagnostic::Severity::Error))
    {
        return Err("SPEC parser errors prevent remote submission".into());
    }
    let materials = crate::check::materials::prepare(
        area.package_directory(),
        &area.sources(),
        &parsed,
        &[],
        false,
    )?;
    let directory = tempfile::tempdir_in(area.directory()).map_err(|e| e.to_string())?;
    for entry in fs_err::read_dir(area.package_directory()).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("non-UTF8 material filename")?;
        if name.starts_with('.') || name == "_service" {
            continue;
        }
        if name.starts_with('_') && name != "_constraints" {
            return Err(format!(
                "reserved OBS file {name}; use _service or _constraints"
            ));
        }
        copy_file(&entry.path(), &directory.path().join(name))?;
    }
    for (name, path) in materials.delivery_files()? {
        copy_file(&path, &directory.path().join(name))?;
    }
    if let Some(text) = constraints {
        let path = directory.path().join("_constraints");
        if path.exists() && crate::utf8_file::read(&path).map_err(|e| e.to_string())? != text {
            return Err("conflicting _constraints in recipe and OBS configuration".into());
        }
        fs_err::write(path, text).map_err(|e| e.to_string())?;
    }
    let constraints_path = directory.path().join("_constraints");
    if constraints_path.exists() {
        validate_constraints(
            &crate::utf8_file::read(&constraints_path).map_err(|e| e.to_string())?,
        )?;
    }
    parse(service)?;
    fs_err::write(directory.path().join("_service"), service).map_err(|e| e.to_string())?;
    if crate::utf8_file::read(&spec).map_err(|e| e.to_string())? != source {
        return Err("SPEC changed during material preparation; retry".into());
    }
    let mut files = BTreeMap::new();
    let mut sha256 = BTreeMap::new();
    for entry in fs_err::read_dir(directory.path()).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or("invalid filename")?
            .to_owned();
        let mut file = fs_err::File::open(entry.path()).map_err(|e| e.to_string())?;
        let mut hash = md5::Context::new();
        let mut buffer = vec![0; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.consume(&buffer[..n]);
        }
        files.insert(name.clone(), format!("{:x}", hash.finalize()));
        sha256.insert(
            name,
            crate::file_digest::read(&entry.path())
                .map_err(|e| e.to_string())?
                .sha256,
        );
    }
    Ok(Input {
        directory,
        files,
        sha256,
    })
}
fn validate_constraints(text: &str) -> Result<(), String> {
    if parse(text)?.root_element().tag_name().name() != "constraints" {
        return Err("_constraints must have a constraints root element".into());
    }
    Ok(())
}
pub(super) fn directory(text: &str) -> Result<BTreeMap<String, String>, String> {
    let doc = parse(text)?;
    if doc.root_element().tag_name().name() != "directory" {
        return Err("OBS did not return a source directory".into());
    }
    let mut files = BTreeMap::new();
    for entry in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("entry"))
    {
        let name = entry
            .attribute("name")
            .ok_or("OBS source entry has no name")?;
        if name.starts_with("_service:") || name.starts_with("_service_") {
            continue;
        }
        let md5 = entry
            .attribute("md5")
            .filter(|s| !s.is_empty())
            .ok_or("OBS source entry has no digest")?;
        if files.insert(name.to_owned(), md5.to_owned()).is_some() {
            return Err(format!("duplicate OBS source entry: {name}"));
        }
    }
    Ok(files)
}
pub(super) fn project(client: &Client, user: &str, settings: &Settings) -> Result<(), String> {
    let project = settings.project.as_deref().ok_or("missing project")?;
    super::api::owned_project(project, user)?;
    let parent = settings.parent.as_deref().ok_or("missing parent")?;
    let metadata = client
        .get(&["source", parent, "_meta"])?
        .ok_or("parent project not found")?;
    let doc = parse(&metadata)?;
    let mut repos = String::new();
    for name in settings
        .repositories
        .as_ref()
        .ok_or("missing repositories")?
    {
        let repo = doc
            .root_element()
            .children()
            .find(|n| n.has_tag_name("repository") && n.attribute("name") == Some(name.as_str()))
            .ok_or_else(|| format!("parent project has no repository {name}"))?;
        write!(
            repos,
            "<repository name=\"{}\"><path project=\"{}\" repository=\"{}\"/>",
            xml(name),
            xml(parent),
            xml(name)
        )
        .expect("String formatting cannot fail");
        for arch in repo.children().filter(|n| n.has_tag_name("arch")) {
            write!(
                repos,
                "<arch>{}</arch>",
                xml(arch.text().ok_or("empty architecture")?)
            )
            .expect("String formatting cannot fail");
        }
        repos.push_str("</repository>");
    }
    let meta = format!(
        "<project name=\"{}\"><title>RuyiPack remote builds</title><description>Managed by RuyiPack</description><person userid=\"{}\" role=\"maintainer\"/><build><enable/></build><publish><{}/></publish>{repos}</project>",
        xml(project),
        xml(user),
        if settings.publish.unwrap_or(false) {
            "enable"
        } else {
            "disable"
        }
    );
    if let Some(old) = client.get(&["source", project, "_meta"])? {
        let old = parse(&old)?;
        if !old
            .descendants()
            .any(|n| n.has_tag_name("description") && n.text() == Some("Managed by RuyiPack"))
        {
            return Err(
                "existing project is not managed by RuyiPack; choose a new home subproject".into(),
            );
        }
        // Metadata changes are explicit configuration changes, not a reason to replace package sources.
    }
    client.put(&["source", project, "_meta"], &[], meta.as_bytes())?;
    Ok(())
}
pub(super) fn submit(
    client: &Client,
    project: &str,
    package: &str,
    input: &Input,
    previous: Option<&Receipt>,
    fresh: bool,
) -> Result<(Receipt, Vec<String>, Vec<String>), String> {
    super::api::identifier(package)?;
    let path = ["source", project, package];
    let current = if let Some(text) = client.get(&path)? {
        directory(&text)?
    } else {
        let meta = format!(
            "<package name=\"{}\" project=\"{}\"><title>{}</title><description>Managed by RuyiPack</description></package>",
            xml(package),
            xml(project),
            xml(package)
        );
        client.put(&["source", project, package, "_meta"], &[], meta.as_bytes())?;
        BTreeMap::new()
    };
    if let Some(previous) = previous {
        if previous.project != project || previous.package != package {
            return Err("remote binding differs from receipt; use a new WORK".into());
        }
        if current != previous.files && current != input.files {
            return Err(
                "remote sources changed outside this WORK; inspect OBS before retrying".into(),
            );
        }
        if !fresh && previous.sha256 != input.sha256 {
            return Err("local materials changed; use --fresh to submit the differences".into());
        }
    } else if !current.is_empty() && current != input.files {
        return Err(
            "remote package has sources but this WORK has no receipt; choose a new project".into(),
        );
    }
    if current.contains_key("_constraints") && !input.files.contains_key("_constraints") {
        return Err("remote _constraints would be removed; configure its local file or remove it explicitly in OBS".into());
    }
    let uploaded: Vec<_> = input
        .files
        .iter()
        .filter(|(n, h)| current.get(*n) != Some(*h))
        .map(|(n, _)| n.clone())
        .collect();
    let removed = current
        .keys()
        .filter(|n| !input.files.contains_key(*n))
        .cloned()
        .collect();
    let mut list = String::from("<directory>");
    for (name, md5) in &input.files {
        write!(
            list,
            "<entry name=\"{}\" md5=\"{}\" hash=\"sha256:{}\"/>",
            xml(name),
            md5,
            input.sha256[name]
        )
        .expect("String formatting cannot fail");
    }
    list.push_str("</directory>");
    if current != input.files {
        for name in &uploaded {
            client.put_file(
                &["source", project, package, name],
                &[("rev", "repository")],
                &input.directory.path().join(name),
            )?;
        }
    }
    let response = if current == input.files {
        client.get(&path)?.ok_or("package disappeared")?
    } else {
        client.post(
            &path,
            &[
                ("cmd", "commitfilelist"),
                ("comment", "ruyipack remote-build"),
            ],
            &list,
        )?
    };
    let doc = parse(&response)?;
    if let Some(error) = doc.root_element().attribute("error") {
        return Err(format!("OBS source commit incomplete: {error}"));
    }
    let revision = doc
        .root_element()
        .attribute("rev")
        .ok_or("OBS commit response has no revision")?
        .to_owned();
    Ok((
        Receipt {
            api: client.origin(),
            project: project.into(),
            package: package.into(),
            revision,
            files: input.files.clone(),
            sha256: input.sha256.clone(),
        },
        uploaded,
        removed,
    ))
}
