// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Stable categories for failures that change how an edit can proceed.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Kind {
    OperationFailed,
    UnmappableFields,
    SourceChanged,
    InvalidDraft,
    InvalidAssignment,
    InvalidCandidate,
    StaticCheckFailed,
}

#[derive(Debug, thiserror::Error, Serialize)]
#[error("{message}")]
pub(crate) struct EditError {
    code: Kind,
    pub(super) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    selected_fields: Vec<String>,
    #[serde(skip)]
    #[source]
    publication: Option<Box<crate::file_output::OutputError>>,
}

impl EditError {
    pub(super) fn at(code: Kind, path: &Path, fields: &[String], message: String) -> Self {
        Self {
            code,
            message,
            path: Some(path.display().to_string()),
            selected_fields: fields.to_vec(),
            publication: None,
        }
    }

    pub(super) fn publication(error: crate::file_output::OutputError) -> Self {
        let mut result = Self::from(error.to_string());
        result.publication = Some(Box::new(error));
        result
    }

    /// Publication facts take precedence over rereading files that may have changed again.
    pub(super) fn invalidates_drafts(&self, sources: &[&Path]) -> bool {
        fn invalidates(error: &crate::file_output::OutputError, sources: &[&Path]) -> bool {
            use crate::file_output::OutputError;
            match error {
                OutputError::Partial { written, source } => {
                    written.iter().any(|path| sources.contains(&path.as_path()))
                        || invalidates(source, sources)
                }
                OutputError::SourceChanged(_) => true,
                _ => false,
            }
        }
        self.publication
            .as_deref()
            .is_some_and(|error| invalidates(error, sources))
    }
}

impl From<String> for EditError {
    fn from(message: String) -> Self {
        Self {
            code: Kind::OperationFailed,
            message,
            path: None,
            selected_fields: Vec::new(),
            publication: None,
        }
    }
}
impl From<&str> for EditError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}
