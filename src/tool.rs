// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared producer identity for machine reports, including early failures.

#[derive(serde::Serialize)]
pub(crate) struct Identity {
    name: &'static str,
    version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dirty: Option<bool>,
}

pub(crate) fn identity() -> Identity {
    Identity {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        revision: Some(env!("RUYIPACK_BUILD_REVISION")).filter(|value| !value.is_empty()),
        dirty: match env!("RUYIPACK_BUILD_DIRTY") {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
    }
}
