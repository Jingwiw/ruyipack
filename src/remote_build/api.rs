// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! OBS XML is a transport boundary; local reports remain TOML.
use base64::Engine;
use std::time::Duration;

pub(super) struct Client {
    base: url::Url,
    reads: std::cell::Cell<usize>,
    auth: String,
    agent: ureq::Agent,
}
impl Client {
    pub(super) fn read_requests(&self) -> usize {
        self.reads.get()
    }

    pub(super) fn origin(&self) -> String {
        self.base.origin().ascii_serialization()
    }

    pub(super) fn new(api: &str, user: &str, password: &str) -> Result<Self, String> {
        let base = url::Url::parse(api).map_err(|e| e.to_string())?;
        if base.scheme() != "https"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err("OBS API must be an HTTPS origin without credentials or query".into());
        }
        Ok(Self {
            base,
            reads: std::cell::Cell::new(0),
            auth: format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
            ),
            agent: ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(15)))
                .timeout_global(Some(Duration::from_secs(300)))
                .max_redirects(0)
                .build()
                .into(),
        })
    }
    fn url(&self, path: &[&str], query: &[(&str, &str)]) -> Result<url::Url, String> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| "invalid API URL")?
            .clear()
            .extend(path);
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        Ok(url)
    }
    pub(super) fn get(&self, path: &[&str]) -> Result<Option<String>, String> {
        self.get_query(path, &[])
    }
    pub(super) fn get_query(
        &self,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<Option<String>, String> {
        let url = self.url(path, query)?;
        self.reads.set(self.reads.get() + 1);
        match self
            .agent
            .get(url.as_str())
            .header("Authorization", &self.auth)
            .call()
        {
            Ok(mut r) => r
                .body_mut()
                .with_config()
                .limit(16 * 1024 * 1024)
                .read_to_string()
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(format!("OBS GET {}: {e}", path.join("/"))),
        }
    }
    pub(super) fn put(
        &self,
        path: &[&str],
        query: &[(&str, &str)],
        bytes: &[u8],
    ) -> Result<String, String> {
        let url = self.url(path, query)?;
        self.agent
            .put(url.as_str())
            .header("Authorization", &self.auth)
            .send(bytes)
            .map_err(|e| format!("OBS PUT {}: {e}", path.join("/")))?
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| e.to_string())
    }
    pub(super) fn put_file(
        &self,
        path: &[&str],
        query: &[(&str, &str)],
        file: &std::path::Path,
    ) -> Result<(), String> {
        let url = self.url(path, query)?;
        let file = std::fs::File::open(file).map_err(|e| e.to_string())?;
        self.agent
            .put(url.as_str())
            .header("Authorization", &self.auth)
            .send(file)
            .map_err(|e| format!("OBS upload {}: {e}", path.join("/")))?;
        Ok(())
    }
    pub(super) fn post(
        &self,
        path: &[&str],
        query: &[(&str, &str)],
        body: &str,
    ) -> Result<String, String> {
        let url = self.url(path, query)?;
        self.agent
            .post(url.as_str())
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/xml")
            .send(body)
            .map_err(|e| format!("OBS POST {}: {e}", path.join("/")))?
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| e.to_string())
    }
}
pub(super) fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub(super) fn parse(s: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse(s).map_err(|e| format!("OBS XML: {e}"))
}
pub(super) fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
    {
        return Err(format!("invalid OBS identifier: {value}"));
    }
    Ok(())
}
pub(super) fn owned_project(project: &str, user: &str) -> Result<(), String> {
    identifier(user)?;
    let suffix = project
        .strip_prefix(&format!("home:{user}:"))
        .ok_or("remote-build may only write below the authenticated user's home:<user>:")?;
    for part in suffix.split(':') {
        identifier(part)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn writes_require_a_real_owned_subproject() {
        assert!(super::owned_project("home:alice:demo", "alice").is_ok());
        for value in [
            "openruyi",
            "home:alice",
            "home:bob:demo",
            "home:alice:",
            "home:alice:../demo",
        ] {
            assert!(super::owned_project(value, "alice").is_err());
        }
        let text = format!("<x name=\"{}\"/>", super::xml("a&\"<b"));
        assert_eq!(
            super::parse(&text)
                .unwrap()
                .root_element()
                .attribute("name"),
            Some("a&\"<b")
        );
    }
}
