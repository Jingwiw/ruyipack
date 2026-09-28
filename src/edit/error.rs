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
    InputRead,
    UnmappableFields,
    SourceChanged,
    SourceHashFailed,
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
    #[serde(flatten, serialize_with = "publication_details")]
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

    pub(super) fn written_paths(&self) -> &[PathBuf] {
        match self.publication.as_deref() {
            Some(crate::file_output::OutputError::Partial { written, .. }) => written,
            _ => &[],
        }
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

// Derive machine details from the same error retained for recovery decisions.
fn publication_details<S: serde::Serializer>(
    error: &Option<Box<crate::file_output::OutputError>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use crate::file_output::OutputError as E;
    use serde_json::json;
    let Some(mut error) = error.as_deref() else {
        return json!({}).serialize(serializer);
    };
    while let E::Partial { source, .. } = error {
        error = source;
    }
    let (reason, path, io) = match error {
        E::Read { path, source } => ("read-failed", Some(path), Some(source)),
        E::Write { path, source } => ("write-failed", Some(path), Some(source)),
        E::Stdout(source) => ("stdout-failed", None, Some(source)),
        E::Stderr(source) => ("stderr-failed", None, Some(source)),
        E::Selection(_) => ("selection-failed", None, None),
        E::Changed(path) => ("target-changed", Some(path), None),
        E::SourceChanged(path) => ("source-changed", Some(path), None),
        E::EditLayout(_) => ("invalid-layout", None, None),
        E::DiffPath(path) => ("invalid-diff-path", Some(path), None),
        E::DiffEncoding { path, .. } => ("invalid-diff-encoding", Some(path), None),
        E::Partial { .. } => unreachable!("unwrapped above"),
    };
    let mut details = json!({"stage": "publication", "reason": reason});
    if let Some(path) = path {
        details["path"] = json!(path);
    }
    if let Some(io) = io {
        use std::io::ErrorKind;
        details["io_kind"] = json!(match io.kind() {
            ErrorKind::PermissionDenied => "permission-denied",
            ErrorKind::NotFound => "not-found",
            ErrorKind::AlreadyExists => "already-exists",
            ErrorKind::IsADirectory => "is-a-directory",
            ErrorKind::NotADirectory => "not-a-directory",
            ErrorKind::ReadOnlyFilesystem => "read-only-filesystem",
            ErrorKind::StorageFull => "storage-full",
            _ => "other",
        });
    }
    details.serialize(serializer)
}
