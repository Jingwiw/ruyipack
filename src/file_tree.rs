// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Copy ordinary recipe trees without following links or overwriting a destination.

use fs_err as fs;
use std::{io, path::Path};

pub(crate) fn copy(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir(destination)?;
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs_err::DirEntry::file_name);
    for entry in entries {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy(&from, &to)?;
        } else if kind.is_file() {
            fs::copy(&from, &to)?;
        } else {
            return Err(io::Error::other(format!(
                "{}: source inputs must not contain symlinks or special files",
                from.display()
            )));
        }
    }
    Ok(())
}
