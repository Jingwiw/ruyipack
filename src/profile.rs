// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Embedded openRuyi defaults shared by SPEC generation and editing.

use serde::Deserialize;

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
pub(crate) fn load() -> Result<Profile, toml::de::Error> {
    toml::from_str(include_str!("../profiles/openruyi-v1/profile.toml"))
}
