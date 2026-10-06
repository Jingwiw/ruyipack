// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Validate the package that will be delivered, not all files present in WORK.

use super::{Pending, baseline};
use crate::{
    build::evidence::{self, Evidence, Inputs},
    workspace::Development,
};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct NameWarning {
    pub(super) directory: String,
    pub(super) literal: Option<String>,
}

#[derive(Serialize)]
pub(super) struct Admission {
    pub(super) allowed: bool,
    pub(super) error: Option<String>,
    pub(super) name_warning: Option<NameWarning>,
    pub(super) spec_fallback: Option<String>,
}

pub(super) fn assess(pending: &Pending, area: &Development) -> (Admission, Option<Evidence>) {
    let mut name_warning = None;
    let mut spec_fallback = None;
    match inputs(pending, area, &mut name_warning, &mut spec_fallback) {
        Ok(inputs) => (
            Admission {
                allowed: true,
                error: None,
                name_warning,
                spec_fallback,
            },
            Some(evidence::compare(
                &area.directory().join("build"),
                area.package(),
                &inputs,
            )),
        ),
        Err(error) => (
            Admission {
                allowed: false,
                error: Some(error),
                name_warning,
                spec_fallback,
            },
            None,
        ),
    }
}

fn inputs(
    pending: &Pending,
    area: &Development,
    warning: &mut Option<NameWarning>,
    fallback: &mut Option<String>,
) -> Result<Inputs, String> {
    let selected = super::super::recipe::select_spec(
        Some(area.package()),
        pending.after.keys().map(std::path::Path::new),
    )
    .map_err(|e| e.to_string())?;
    if area.spec().map_err(|e| e.to_string())?.file_name() != selected.file_name() {
        return Err("delivery selects a different SPEC than WORK; reconcile the SPEC filenames before committing".into());
    }
    let spec = super::git::text(&selected)
        .map_err(|e| e.to_string())?
        .to_owned();
    if spec != format!("{}.spec", area.package()) {
        *fallback = Some(spec.clone());
    }
    let identity = pending
        .after
        .get(&spec)
        .ok_or("delivery does not contain the bound SPEC")?;
    let root = if pending.before.get(&spec) == Some(identity) {
        pending.repository.join(&pending.package)
    } else {
        area.package_directory().to_path_buf()
    };
    let text = crate::utf8_file::read(&root.join(&spec)).map_err(|e| e.to_string())?;
    if crate::utf8_file::sha256(&text) != identity.sha256 {
        return Err("SPEC changed during delivery validation".into());
    }
    let parsed = crate::spec::ParsedSpec::parse(&text);
    if let Some(previous) = pending.before.get(&spec) {
        let original =
            crate::utf8_file::read(&pending.repository.join(&pending.package).join(&spec))
                .map_err(|e| e.to_string())?;
        if crate::utf8_file::sha256(&original) == previous.sha256 {
            crate::spec::sources::check_removal(
                &crate::spec::ParsedSpec::parse(&original),
                &parsed,
            )?;
        }
    }

    let literal = super::field(&parsed, "name");
    if literal.as_deref() != Some(area.package()) {
        *warning = Some(NameWarning {
            directory: area.package().to_owned(),
            literal,
        });
    }
    if !crate::check::analyze(&parsed, crate::check::Policy::Submit, &[]).is_success() {
        return Err("SPEC failed submit checks; run check WORK --policy submit. Repository files were not changed by this attempt".into());
    }
    let mut inputs = Inputs::from([(format!("SPECS/{spec}"), (identity.sha256.clone(), None))]);
    for material in crate::check::materials::requirements(&parsed)? {
        baseline::relative(&material.name).map_err(|e| e.to_string())?;
        let file = pending.after.get(&material.name);
        if file.is_none() && !material.remote {
            return Err(format!(
                "{}: {} is missing from the delivery; include the material or change the SPEC",
                material.identity, material.name
            ));
        }
        if material.remote && material.sha256.is_none() {
            return Err(format!(
                "{}: remote material needs a declared SHA-256",
                material.identity
            ));
        }
        let hash = material
            .sha256
            .or_else(|| file.map(|file| file.sha256.clone()))
            .ok_or_else(|| {
                format!(
                    "{}: remote material needs a declared SHA-256",
                    material.identity
                )
            })?;
        if file.is_some_and(|file| !hash.eq_ignore_ascii_case(&file.sha256)) {
            return Err(format!(
                "{}: {} does not match its declared SHA-256 in the delivery",
                material.identity, material.name
            ));
        }
        let expected = (hash, file.map(|file| file.executable));
        if let Some(previous) =
            inputs.insert(format!("SOURCES/{}", material.name), expected.clone())
            && !previous.0.eq_ignore_ascii_case(&expected.0)
        {
            return Err(format!("{}: conflicting material hashes", material.name));
        }
    }
    Ok(inputs)
}
