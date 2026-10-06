// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

use serde::{Deserialize, Serialize};
use std::{
    io::{IsTerminal, Write},
    path::Path,
};

pub(super) const SERVICE: &str =
    "<services>\n  <service name=\"download_assets\" mode=\"trylocal\"/>\n</services>\n";
pub(super) use crate::plan::{Plan, Settings};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Global {
    pub api: String,
    #[serde(flatten)]
    pub defaults: Settings,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Auth {
    pub user: String,
    pub password: String,
}
pub(super) fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    toml::from_str(&crate::utf8_file::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
pub(super) fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = toml::to_string_pretty(value).map_err(|e| e.to_string())?;
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("configuration has no parent")?)
            .map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub(super) fn load(root: &Path, interactive: bool) -> Result<(Global, Auth), String> {
    let path = root.join("obs.toml");
    if !path.exists() {
        save(
            &path,
            &Global {
                api: "https://build.openruyi.cn".into(),
                defaults: Settings {
                    parent: Some("openruyi".into()),
                    repositories: Some(vec!["x86_64".into()]),
                    publish: Some(false),
                    ..Settings::default()
                },
            },
        )?;
    }
    let global: Global = read(&path)?;
    let auth_path = root.join("obs-auth.toml");
    let ignore = root.join(".gitignore");
    let mut contents = match fs_err::read_to_string(&ignore) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    if !contents.lines().any(|line| line == "/obs-auth.toml") {
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str("/obs-auth.toml\n");
        fs_err::write(&ignore, contents).map_err(|e| e.to_string())?;
    }

    let mut auth: Auth = if auth_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::metadata(&auth_path)
                .map_err(|e| e.to_string())?
                .permissions()
                .mode()
                & 0o077
                != 0
            {
                return Err("obs-auth.toml must have permissions 0600".into());
            }
        }
        read(&auth_path)?
    } else {
        if !interactive || !std::io::stdin().is_terminal() {
            return Err(format!(
                "set user and password in {} (mode 0600), or run remote-build interactively",
                auth_path.display()
            ));
        }
        let user = crate::prompt::value("OBS user", "").map_err(|e| e.to_string())?;
        let password = inquire::Password::new("OBS password:")
            .without_confirmation()
            .prompt()
            .map_err(|e| e.to_string())?;
        let auth = Auth { user, password };
        save(&auth_path, &auth)?;
        auth
    };
    if auth.user.is_empty() || auth.password.is_empty() {
        if !interactive || !std::io::stdin().is_terminal() {
            return Err("OBS user/password missing in obs-auth.toml".into());
        }
        if auth.user.is_empty() {
            auth.user = crate::prompt::value("OBS user", "").map_err(|e| e.to_string())?;
        }
        if auth.password.is_empty() {
            auth.password = inquire::Password::new("OBS password:")
                .without_confirmation()
                .prompt()
                .map_err(|e| e.to_string())?;
        }
        save(&auth_path, &auth)?;
    }
    super::api::identifier(&auth.user)?;
    if auth.password.is_empty() {
        return Err("OBS password is empty".into());
    }
    Ok((global, auth))
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
