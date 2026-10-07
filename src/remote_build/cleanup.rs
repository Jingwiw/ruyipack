// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Delete receipt-bound package sources, never a shared OBS project.
use super::{api, config, delivery};
use crate::workspace::{Development, Workspace};
use serde::Serialize;

#[derive(Default, Serialize)]
pub(crate) struct Report {
    pub(crate) target: Option<String>,
    pub(crate) package_removed: bool,
    already_absent: bool,
    pub(crate) retained: Vec<String>,
}

pub(crate) fn execute(
    workspace: &Workspace,
    area: &Development,
    preview: bool,
    report: &mut Report,
) -> Result<(), String> {
    let path = area.directory().join("remote-receipt.toml");
    if !path.try_exists().map_err(|e| e.to_string())? {
        if area
            .directory()
            .join("remote.toml")
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err(
                "OBS settings have no receipt; reconcile with remote-build before deletion".into(),
            );
        }
        return Ok(());
    }
    let receipt: delivery::Receipt = config::read(&path)?;
    if receipt.package != area.package() {
        return Err("OBS receipt package differs from WORK binding".into());
    }
    api::identifier(&receipt.package)?;
    report.target = Some(format!(
        "{}/source/{}/{}",
        receipt.api, receipt.project, receipt.package
    ));
    report.retained.push(format!(
        "project {}: may have other consumers",
        receipt.project
    ));
    let _lock = if preview {
        None
    } else {
        Some(config::lock(&workspace.configuration())?)
    };
    if shared_package(workspace, area, &receipt)? {
        report.retained.push(format!(
            "package {}: another WORK uses this binding",
            receipt.package
        ));
        return Ok(());
    }
    if preview {
        return Ok(());
    }
    let (global, auth) = config::load(&workspace.configuration(), false)?;
    let client = api::Client::new(&global.api, &auth.user, &auth.password)?;
    if client.origin() != receipt.api {
        return Err("configured OBS API differs from retained receipt".into());
    }
    api::owned_project(&receipt.project, &auth.user)?;
    let metadata = client.get(&["source", &receipt.project, "_meta"])?;
    if let Some(metadata) = metadata {
        if !api::parse(&metadata)?
            .root_element()
            .children()
            .any(|n| n.has_tag_name("description") && n.text() == Some("Managed by RuyiPack"))
        {
            return Err("OBS project is not managed by RuyiPack".into());
        }
        let target = ["source", &receipt.project, &receipt.package];
        if let Some(current) = client.get(&target)? {
            if delivery::directory(&current)? != receipt.files {
                return Err("OBS package differs from retained receipt; nothing deleted".into());
            }
            client.delete(&target)?;
            report.package_removed = true;
        } else {
            report.already_absent = true;
        }
    } else {
        report.already_absent = true;
    }
    // Keep the receipt until all other binding files are removed, so retry can reconcile a 404.
    for name in ["remote.toml", "remote-result.toml", "remote-receipt.toml"] {
        match fs_err::remove_file(area.directory().join(name)) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn shared_package(
    workspace: &Workspace,
    area: &Development,
    receipt: &delivery::Receipt,
) -> Result<bool, String> {
    let root = area.directory().parent().ok_or("WORK has no parent")?;
    for entry in fs_err::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path() == area.directory()
            || !entry.file_type().map_err(|e| e.to_string())?.is_dir()
        {
            continue;
        }
        let retained = entry.path().join("remote-receipt.toml");
        if retained.try_exists().map_err(|e| e.to_string())? {
            let other: delivery::Receipt = config::read(&retained)?;
            if (&other.api, &other.project, &other.package)
                == (&receipt.api, &receipt.project, &receipt.package)
            {
                return Ok(true);
            }
        }
        let configured = entry.path().join("remote.toml");
        if configured.try_exists().map_err(|e| e.to_string())? {
            let settings: config::Settings = config::read(&configured)?;
            if settings.project.as_deref() == Some(&receipt.project) {
                let name = entry.file_name();
                let other = workspace
                    .existing_development(name.to_str().ok_or("non-UTF8 WORK")?)
                    .map_err(|e| e.to_string())?;
                if other.package() == receipt.package {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
