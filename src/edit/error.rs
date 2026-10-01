// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Stable categories for failures that change how an edit can proceed.

use serde::Serialize;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

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
#[error("{}", self.display_message())]
pub(crate) struct EditError {
    code: Kind,
    pub(super) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    selected_fields: Vec<String>,
    #[serde(flatten)]
    #[source]
    cause: Option<Box<Cause>>,
}

#[derive(Debug, thiserror::Error)]
enum Cause {
    #[error("{0}")]
    Publication(#[source] crate::file_output::OutputError),
    #[error("{0}")]
    SourceHash(#[source] crate::source::Error),
}

impl EditError {
    // Presentation only: keep serialized evidence and typed causes unchanged.
    fn display_message(&self) -> Cow<'_, str> {
        let Some(path) = &self.path else {
            return Cow::Borrowed(&self.message);
        };
        let Some(suffix) = self
            .message
            .strip_prefix(path)
            .filter(|suffix| suffix.starts_with(':'))
        else {
            return Cow::Borrowed(&self.message);
        };
        Cow::Owned(format!(
            "{}{suffix}",
            crate::output_cli::human_path(Path::new(path)).display()
        ))
    }

    pub(super) fn at(code: Kind, path: &Path, fields: &[String], message: String) -> Self {
        Self {
            code,
            message,
            path: Some(path.display().to_string()),
            selected_fields: fields.to_vec(),
            cause: None,
        }
    }

    pub(super) fn source_hash(error: crate::source::Error, path: &Path, fields: &[String]) -> Self {
        let mut result = Self::at(Kind::SourceHashFailed, path, fields, error.to_string());
        result.cause = Some(Box::new(Cause::SourceHash(error)));
        result
    }

    pub(super) fn publication(error: crate::file_output::OutputError) -> Self {
        let mut result = Self::from(error.to_string());
        result.cause = Some(Box::new(Cause::Publication(error)));
        result
    }

    pub(super) fn written_paths(&self) -> &[PathBuf] {
        match self.cause.as_deref() {
            Some(Cause::Publication(crate::file_output::OutputError::Partial {
                written, ..
            })) => written,
            _ => &[],
        }
    }
}

impl From<String> for EditError {
    fn from(message: String) -> Self {
        Self {
            code: Kind::OperationFailed,
            message,
            path: None,
            selected_fields: Vec::new(),
            cause: None,
        }
    }
}
impl From<&str> for EditError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

// Derive machine details from the same error retained for recovery decisions.
impl Serialize for Cause {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use crate::file_output::OutputError as E;
        #[derive(Serialize)]
        struct Publication<'a> {
            stage: &'static str,
            reason: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            path: Option<Cow<'a, str>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            io_kind: Option<&'static str>,
        }
        let mut error = match self {
            Self::SourceHash(error) => return error.details().serialize(serializer),
            Self::Publication(error) => error,
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
        let io_kind = io.map(|io| {
            use std::io::ErrorKind;
            match io.kind() {
                ErrorKind::PermissionDenied => "permission-denied",
                ErrorKind::NotFound => "not-found",
                ErrorKind::AlreadyExists => "already-exists",
                ErrorKind::IsADirectory => "is-a-directory",
                ErrorKind::NotADirectory => "not-a-directory",
                ErrorKind::ReadOnlyFilesystem => "read-only-filesystem",
                ErrorKind::StorageFull => "storage-full",
                _ => "other",
            }
        });
        Publication {
            stage: "publication",
            reason,
            path: path.map(|path| path.to_string_lossy()),
            io_kind,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn human_paths_are_relative_without_rewriting_report_evidence() {
        let path = std::env::current_dir().unwrap().join("example.spec");
        let message = format!("{}: candidate failed static checks", path.display());
        let error =
            super::EditError::at(super::Kind::StaticCheckFailed, &path, &[], message.clone());
        assert_eq!(
            error.to_string(),
            "example.spec: candidate failed static checks"
        );
        let report: toml::Table = toml::from_str(&toml::to_string(&error).unwrap()).unwrap();
        assert_eq!(report["message"].as_str(), Some(message.as_str()));
        assert_eq!(
            report["path"].as_str(),
            Some(path.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn source_hash_failure_retains_its_typed_cause() {
        let error = super::EditError::source_hash(
            crate::source::Error::resolution("unresolved"),
            std::path::Path::new("package.spec"),
            &[],
        );
        assert!(
            std::iter::successors(Some(&error as &dyn std::error::Error), |e| e.source())
                .any(<dyn std::error::Error>::is::<crate::source::Error>)
        );
    }
}
