// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Embedded defaults are materialized as an ordinary Compose build context.

use fs_err as fs;
use std::{io, path::Path};

const FILES: &[(&str, &[u8])] = &[
    (
        "Dockerfile",
        include_bytes!("../environments/openruyi/Dockerfile"),
    ),
    (
        "compose.yaml",
        include_bytes!("../environments/openruyi/compose.yaml"),
    ),
    (
        "openruyi.cfg",
        include_bytes!("../environments/openruyi/openruyi.cfg"),
    ),
    (
        "target.json",
        include_bytes!("../environments/openruyi/target.json"),
    ),
];

pub(crate) fn write(directory: &Path) -> io::Result<()> {
    fs::create_dir(directory)?;
    for (name, bytes) in FILES {
        fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}
