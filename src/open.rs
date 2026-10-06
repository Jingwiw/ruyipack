// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Open the bound recipe or authoring input without staging or publication.

use clap::Args;

#[derive(Args)]
#[command(
    after_help = "Open recipe files, or select --authoring to open the TOML input.\nThe editor saves files directly. Check and commit changes separately.\nAfter TOML edits: ruyipack gen WORK --diff\nAfter recipe edits: ruyipack check WORK; ruyipack build WORK\nGUI editors must wait: --editor 'code --wait'."
)]
pub(crate) struct Options {
    /// Development area to open.
    work: String,
    /// Bind a new development area to this package.
    #[arg(long, value_name = "PKG")]
    pkgname: Option<String>,
    /// Open the existing authoring TOML instead of the recipe directory; does not generate files.
    #[arg(long, conflicts_with = "pkgname")]
    authoring: bool,
    /// Override the configured editor; GUI editors must wait.
    #[arg(long, value_name = "COMMAND")]
    editor: Option<String>,
}

pub(crate) fn run(options: &Options) -> std::io::Result<bool> {
    let workspace = crate::workspace::discover()?;
    if options.authoring {
        let area = workspace.existing_development(&options.work)?;
        let manifest = area.manifest();
        if !fs_err::symlink_metadata(&manifest)?.is_file() {
            return Err(std::io::Error::other(
                "authoring TOML must be a regular file",
            ));
        }
        return crate::editor::open(&[&manifest], options.editor.as_deref(), area.directory())
            .map(|()| true)
            .map_err(std::io::Error::other);
    }
    let mut area = workspace.development(&options.work, options.pkgname.as_deref(), false)?;
    area.create()?;
    let recipe = area.editor_directory();
    crate::editor::open(&[&recipe], options.editor.as_deref(), &recipe)
        .map_err(std::io::Error::other)?;
    Ok(true)
}
