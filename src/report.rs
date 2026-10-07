// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! TOML report facts shared by the commands; no input-format or CLI policy.

use serde::{Serialize, Serializer};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{self, Write},
};

#[derive(Serialize)]
pub(crate) struct Input<'a> {
    pub(crate) display_path: Cow<'a, str>,
    pub(crate) sha256: Option<&'a str>,
    pub(crate) revision: Option<&'a str>,
}

/// Material numbers are data in TOML arrays. Map iteration retains numeric order;
/// explicit sequences (notably Patch application order) use Entry directly.
#[derive(Serialize)]
pub(crate) struct Entry<'a, T> {
    pub(crate) number: u32,
    #[serde(flatten)]
    pub(crate) value: &'a T,
}

pub(crate) struct Numbered<'a, T>(pub(crate) &'a BTreeMap<u32, T>);

impl<T: Serialize> Serialize for Numbered<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(
            self.0
                .iter()
                .map(|(&number, value)| Entry { number, value }),
        )
    }
}

pub(crate) fn numbered<T: Serialize, S: Serializer>(
    map: &BTreeMap<u32, T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    Numbered(map).serialize(serializer)
}

/// Shared error shape; the calling boundary owns the domain-specific code.
#[derive(serde::Serialize)]
pub(crate) struct Failure {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

pub(crate) fn failure(code: &'static str, message: impl std::fmt::Display) -> Failure {
    Failure {
        code,
        message: message.to_string(),
    }
}

#[derive(Serialize)]
pub(crate) struct FailureReport<'a> {
    format_version: u32,
    valid: bool,
    tool: crate::tool::Identity,
    input: Input<'a>,
    error: Failure,
}

pub(crate) fn failed(input: Input<'_>, error: Failure) -> FailureReport<'_> {
    FailureReport {
        format_version: 2,
        valid: false,
        tool: crate::tool::identity(),
        input,
        error,
    }
}

/// Serialize before writing so a malformed report cannot leave partial TOML.
pub(crate) fn write(writer: &mut impl Write, report: &impl serde::Serialize) -> io::Result<()> {
    let document = toml::to_string_pretty(report).map_err(io::Error::other)?;
    writer.write_all(document.as_bytes())?;
    if !document.ends_with('\n') {
        writeln!(writer)?;
    }
    Ok(())
}
