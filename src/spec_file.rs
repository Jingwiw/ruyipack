// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Strict UTF-8 input for RPM SPEC commands.

use std::{
    error, fmt, fs, io,
    path::{Path, PathBuf},
    string::FromUtf8Error,
};

/// Reads one SPEC file as UTF-8 text.
pub(crate) fn read(path: &Path) -> Result<String, SpecReadError> {
    let bytes = fs::read(path).map_err(|source| SpecReadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    String::from_utf8(bytes).map_err(|source| SpecReadError::Utf8 {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug)]
pub(crate) enum SpecReadError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Utf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
}

impl fmt::Display for SpecReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::Utf8 { path, source } => {
                write!(formatter, "{} is not UTF-8: {source}", path.display())
            }
        }
    }
}

impl error::Error for SpecReadError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Utf8 { source, .. } => Some(source),
        }
    }
}
