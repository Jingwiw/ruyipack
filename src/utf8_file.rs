// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! UTF-8 input and source-change checks shared by file-based commands.

use std::{
    fs, io,
    path::{Path, PathBuf},
    string::FromUtf8Error,
};

/// Identifies the exact UTF-8 bytes, without normalization.
pub(crate) fn digest(source: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

/// Reads one file as UTF-8 text.
pub(crate) fn read(path: &Path) -> Result<String, Utf8FileError> {
    let bytes = fs::read(path).map_err(|source| Utf8FileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    String::from_utf8(bytes).map_err(|source| Utf8FileError::Utf8 {
        path: path.to_path_buf(),
        source,
    })
}

/// Checks that a previously resolved source path and its bytes are unchanged.
pub(crate) fn is_unchanged(path: &Path, original: &str) -> io::Result<bool> {
    Ok(fs::canonicalize(path)? == path && fs::read(path)? == original.as_bytes())
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Utf8FileError {
    #[error("failed to read {}: {source}", .path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} is not UTF-8: {source}", .path.display())]
    Utf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
}
