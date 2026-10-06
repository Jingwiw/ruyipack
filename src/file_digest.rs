// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Bounded SHA-256 snapshots of regular files shared by preflight and builds.

use fs_err as fs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Content {
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("{}: expected a regular file, not a symlink or special file", .0.display())]
    NotRegular(PathBuf),
    #[error("{} changed while hashing; retry against stable files", .0.display())]
    Changed(PathBuf),
}

/// Reject links/FIFOs before opening, cap growth, and detect concurrent changes.
/// This is a stable-workspace check, not an untrusted-directory race sandbox.
pub(crate) fn read(path: &Path) -> Result<Content, Error> {
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() {
        return Err(Error::NotRegular(path.to_owned()));
    }
    let mut file = fs::File::open(path)?.take(before.len().saturating_add(1));
    let mut hash = Sha256::new();
    let size = io::copy(&mut file, &mut hash)?;
    let after = fs::symlink_metadata(path)?;
    if !after.is_file()
        || size != before.len()
        || after.len() != before.len()
        || after.modified()? != before.modified()?
    {
        return Err(Error::Changed(path.to_owned()));
    }
    Ok(Content {
        size,
        sha256: format!("{:x}", hash.finalize()),
    })
}

/// Git and staged build inputs preserve the executable bit, not platform permission masks.
pub(crate) fn executable(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}
