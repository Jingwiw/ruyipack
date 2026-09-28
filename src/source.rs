// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source URL policy and downloads, shared by generation and editing.
//! Static resolution never executes macros; downloads hash the response stream.

use crate::{spec, utf8_file};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    io::Read,
    sync::LazyLock,
    time::{Duration, Instant},
};
use url::{SyntaxViolation, Url};

/// Checks a Source using only already-known package fields, preserving its spelling.
/// RPM syntax is recognized before URL validation, including escaped literal percent signs.
pub(crate) fn validate_expression(value: &str, fields: &[(&str, &str)]) -> Result<Url, String> {
    let resolved = crate::spec::expression::substitute_fields(value, fields)?;
    validate_authoring_url(&resolved)
}

/// Checks URL syntax independently of the generator's HTTPS-only publishing policy.
pub(crate) fn validate_url(value: &str) -> Result<Url, String> {
    let invalid = || {
        "expected an absolute HTTP or HTTPS URL without whitespace or repaired syntax".to_owned()
    };
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    let repaired = Cell::new(false);
    let capture = |violation| {
        if matches!(
            violation,
            SyntaxViolation::ExpectedDoubleSlash
                | SyntaxViolation::Backslash
                | SyntaxViolation::NonUrlCodePoint
                | SyntaxViolation::PercentDecode
        ) {
            repaired.set(true);
        }
    };
    let parsed = Url::options()
        .syntax_violation_callback(Some(&capture))
        .parse(value)
        .map_err(|_| invalid())?;
    if repaired.get() || parsed.host().is_none() {
        return Err(invalid());
    }
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(invalid());
    }
    Ok(parsed)
}

/// Parse once, then apply authoring policy without exposing credentials in errors.
pub(crate) fn validate_authoring_url(value: &str) -> Result<Url, String> {
    let url = validate_url(value)?;
    credentials(&url)?;
    Ok(url)
}

/// Existing package URLs may contain unresolved macros; syntax checks own those results.
pub(crate) fn reject_credentials(value: &str) -> Result<(), String> {
    Url::parse(value).map_or(Ok(()), |url| credentials(&url))
}

fn credentials(url: &Url) -> Result<(), String> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "URL credentials are not allowed; supply authentication outside the SPEC".into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_sha256(value: &str) -> Result<(), &'static str> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("expected 64 hexadecimal digits")
    }
}

/// New manifests require HTTPS; existing SPEC editing also accepts HTTP.
pub(crate) fn require_https(field: &str, url: Url) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err(format!("{field}: expected an HTTPS URL"));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub(crate) struct Download {
    resolved_url: String,
    effective_url: String,
    pub(crate) sha256: String,
    bytes: u64,
}

#[derive(Serialize)]
pub(crate) struct SourceHashes {
    pub(crate) input_sha256: String,
    pub(crate) defines: Vec<String>,
    pub(crate) sources: std::collections::BTreeMap<u32, Download>,
}

/// Resolve the complete selection before starting any downloads.
pub(crate) fn calculate(
    contents: &str,
    numbers: &[u32],
    defines: &[String],
) -> Result<SourceHashes, String> {
    let parsed = spec::ParsedSpec::parse(contents);
    let resolved = spec::sources::resolve(&parsed, defines)?;
    if let Some(reason) = resolved.incomplete {
        return Err(reason);
    }
    let sources = resolved.sources;
    let urls = numbers
        .iter()
        .map(|number| {
            let source = sources
                .get(number)
                .ok_or_else(|| format!("Source{number}: declaration unavailable"))?;
            let url = source
                .url
                .as_ref()
                .map_err(|e| format!("Source{number}: {e}"))?;
            RemoteSource::parse(url).map(|url| (*number, url))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sources = urls
        .into_iter()
        .map(|(number, url)| {
            url.download()
                .map(|download| (number, download))
                .map_err(|e| format!("Source{number}: {e}"))
        })
        .collect::<Result<_, _>>()?;
    Ok(SourceHashes {
        input_sha256: utf8_file::sha256(contents),
        defines: defines.to_vec(),
        sources,
    })
}

/// A checked remote URL retains its original spelling for evidence.
/// The URL fragment is RPM archive naming metadata, not an HTTP request component.
pub(crate) struct RemoteSource<'url> {
    original: &'url str,
    url: Url,
}

impl<'url> RemoteSource<'url> {
    pub(crate) fn parse(original: &'url str) -> Result<Self, String> {
        Ok(Self {
            original,
            url: validate_authoring_url(original)?,
        })
    }

    /// Hash archive bytes directly. No HTTP content decoding or temporary archive.
    pub(crate) fn download(self) -> Result<Download, String> {
        static CLIENT: LazyLock<Result<ureq::Agent, String>> = LazyLock::new(|| {
            let mut tls = ureq::tls::TlsConfig::builder();
            // Respect an explicit CA bundle without silently disabling TLS validation.
            if let Some(path) = std::env::var_os("SSL_CERT_FILE") {
                let pem = std::fs::read(&path).map_err(|e| format!("SSL_CERT_FILE: {e}"))?;
                let certs = ureq::tls::parse_pem(&pem)
                    .filter_map(|item| match item {
                        Ok(ureq::tls::PemItem::Certificate(cert)) => Some(Ok(cert)),
                        Ok(_) => None,
                        Err(e) => Some(Err(e.to_string())),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if certs.is_empty() {
                    return Err("SSL_CERT_FILE: no certificates".into());
                }
                tls = tls.root_certs(certs.into());
            }
            Ok(ureq::Agent::config_builder()
                .tls_config(tls.build())
                .timeout_connect(Some(Duration::from_secs(10)))
                .max_redirects(0)
                .accept_encoding("identity")
                .build()
                .into())
        });
        let agent = CLIENT.as_ref().map_err(Clone::clone)?;
        self.download_with(agent, Duration::from_secs(300))
    }

    fn download_with(self, agent: &ureq::Agent, budget: Duration) -> Result<Download, String> {
        // Retries, redirects and body reads spend the same per-source budget.
        let deadline = Instant::now() + budget;
        let result = match self.attempt(agent, deadline) {
            Err(error) if error.retryable && Instant::now() < deadline => {
                std::thread::sleep(
                    Duration::from_millis(100)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
                self.attempt(agent, deadline)
            }
            result => result,
        };
        result.map_err(|error| error.message)
    }

    fn attempt(&self, agent: &ureq::Agent, deadline: Instant) -> Result<Download, DownloadError> {
        let mut url = self.url.clone();
        url.set_fragment(None);
        let mut redirects = 0;
        let mut response = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("download: total timeout".into());
            }
            let response = agent
                .get(url.as_str())
                .config()
                .timeout_global(Some(remaining))
                .build()
                .call()
                .map_err(DownloadError::request)?;
            match response.status().as_u16() {
                301 | 302 | 303 | 307 | 308 => {
                    if redirects == 10 {
                        return Err("download: more than 10 redirects".into());
                    }
                    let location = response
                        .headers()
                        .get("location")
                        .and_then(|value| value.to_str().ok())
                        .ok_or("download: redirect has no valid Location")?;
                    let next = url
                        .join(location)
                        .map_err(|_| "download: invalid redirect URL")?;
                    let mut next = validate_authoring_url(next.as_str())?;
                    if url.scheme() == "https" && next.scheme() != "https" {
                        return Err("download: HTTPS redirect would downgrade to HTTP".into());
                    }
                    next.set_fragment(None);
                    url = next;
                    redirects += 1;
                }
                200 => break response,
                _ => {
                    return Err(format!(
                        "download: expected HTTP 200, received {}",
                        response.status()
                    )
                    .into());
                }
            }
        };
        let effective_url = url.to_string();
        let mut reader = response.body_mut().as_reader();
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if Instant::now() >= deadline {
                return Err("download body: total timeout".into());
            }
            let length = reader.read(&mut buffer).map_err(|e| DownloadError {
                message: format!("download body: {e}"),
                retryable: true,
            })?;
            if length == 0 {
                break;
            }
            digest.update(&buffer[..length]);
            bytes += length as u64;
        }
        Ok(Download {
            resolved_url: self.original.to_owned(),
            effective_url,
            sha256: format!("{:x}", digest.finalize()),
            bytes,
        })
    }
}

/// Only transient transport/status failures may retry; policy and TLS failures may not.
struct DownloadError {
    message: String,
    retryable: bool,
}

impl DownloadError {
    fn request(error: ureq::Error) -> Self {
        let retryable = matches!(
            error,
            ureq::Error::Io(_)
                | ureq::Error::Timeout(_)
                | ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed
                | ureq::Error::BodyStalled
                | ureq::Error::StatusCode(408 | 429 | 502 | 503 | 504)
        );
        Self {
            message: format!("download: {error}"),
            retryable,
        }
    }
}

impl From<String> for DownloadError {
    fn from(message: String) -> Self {
        Self {
            message,
            retryable: false,
        }
    }
}

impl From<&str> for DownloadError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn slow_headers_and_bodies_share_a_deadline_without_partial_hashes() {
        let agent: ureq::Agent = ureq::Agent::config_builder().proxy(None).build().into();
        for headers in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/archive", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                if headers {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                        .unwrap();
                    socket.flush().unwrap();
                }
                thread::sleep(Duration::from_millis(700));
            });
            let start = Instant::now();
            let result = RemoteSource::parse(&url)
                .unwrap()
                .download_with(&agent, Duration::from_millis(100));
            let elapsed = start.elapsed();
            assert!(
                result
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.to_lowercase().contains("timeout")),
                "{result:?}"
            );
            assert!(
                elapsed < Duration::from_millis(600),
                "request did not stop before the peer: {elapsed:?}"
            );
            server.join().unwrap();
        }
    }
}
