// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Embedded openRuyi defaults shared by SPEC generation and editing.
//!
//! `profiles/openruyi-v1/profile.toml` pins the packaging-guideline revision;
//! see `docs/design.md#openruyi-policy-sources` for the upstream document.
//! Its license applies to the SPEC file, not the packaged software. Release and
//! changelog macros are emitted literally for the target RPM environment.
//! RemoteAsset is openRuyi source-fetch metadata carried in RPM comment lines.

pub(crate) mod buildsystems;

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const SOURCE: &str = include_str!("../profiles/openruyi-v1/profile.toml");

#[derive(Serialize)]
pub(crate) struct Identity {
    pub(crate) name: String,
    pub(crate) sha256: String,
}

// Hash the shipped TOML, including comments; this is provenance, not a semantic
// policy version or the identity of every rule in this executable.
pub(crate) fn identity() -> Identity {
    Identity {
        name: "openruyi-v1".into(),
        sha256: crate::utf8_file::digest(SOURCE),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Profile {
    pub(crate) spec_license: String,
    pub(crate) copyright_holders: Vec<String>,
    pub(crate) release: String,
    pub(crate) changelog: String,
    pub(crate) no_public_vcs_comment: String,
    pub(crate) remote_asset_prefix: String,
    pub(crate) remote_asset_bare: String,
    pub(crate) preamble_value_column: usize,
}

impl Profile {
    /// The RemoteAsset marker, including a digest when supplied.
    pub(crate) fn remote_asset(&self, digest: Option<&str>) -> String {
        match digest {
            Some(hash) => format!("{}{hash}", self.remote_asset_prefix),
            None => self.remote_asset_bare.clone(),
        }
    }
}
pub(crate) fn load() -> &'static Profile {
    static PROFILE: OnceLock<Profile> = OnceLock::new();
    PROFILE.get_or_init(|| toml::from_str(SOURCE).expect("embedded profile must be valid"))
}
