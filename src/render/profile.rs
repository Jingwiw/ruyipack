// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Embedded openRuyi rendering policy and build-system selection.

use super::{RenderError, manifest::Manifest};
use crate::profile::Profile;
/// Selects a supported rendering system; shared checks enforce its requirements.
pub(crate) fn load(manifest: &Manifest) -> Result<Profile, RenderError> {
    let profile = crate::profile::load()?;
    let Some(system) = manifest.build.system.as_deref() else {
        return Ok(profile);
    };
    if crate::check::build::contract(system).is_none() {
        return Err(RenderError::Invalid(format!(
            "build.system: unsupported build system {system:?}"
        )));
    }
    Ok(profile)
}
