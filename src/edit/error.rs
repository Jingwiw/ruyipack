// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Stable categories for failures that change how an edit can proceed.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Kind {
    OperationFailed,
    UnmappableFields,
    SourceChanged,
    InvalidDraft,
    DraftShape,
    InvalidAssignment,
    InvalidCandidate,
    StaticCheckFailed,
    PublicationFailed,
    PartialPublication,
}

#[derive(Debug, thiserror::Error, Serialize)]
#[error("{message}")]
pub(crate) struct EditError {
    code: Kind,
    pub(super) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    selected_fields: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    written: Vec<PathBuf>,
}

impl EditError {
    pub(super) fn at(code: Kind, path: &Path, fields: &[String], message: String) -> Self {
        Self {
            code,
            message,
            path: Some(path.to_owned()),
            selected_fields: fields.to_vec(),
            written: Vec::new(),
        }
    }

    pub(super) fn publication(error: crate::file_output::OutputError) -> Self {
        use crate::file_output::OutputError;
        let mut result = Self::from(error.to_string());
        result.code = Kind::PublicationFailed;
        match error {
            OutputError::Partial { written, .. } => {
                result.code = Kind::PartialPublication;
                result.written = written;
            }
            OutputError::SourceChanged(path) => {
                result.code = Kind::SourceChanged;
                result.path = Some(path);
            }
            _ => {}
        }
        result
    }
}

impl From<String> for EditError {
    fn from(message: String) -> Self {
        Self {
            code: Kind::OperationFailed,
            message,
            path: None,
            selected_fields: Vec::new(),
            written: Vec::new(),
        }
    }
}
impl From<&str> for EditError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}
