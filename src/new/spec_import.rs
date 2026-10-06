// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Import original recipe bytes; the editable projection is not a full SPEC conversion.

use super::{NewError, Options};
use crate::{
    draft, file_output,
    spec::{ParsedSpec, document::Snapshot},
    workspace,
};
use fs_err as fs;
use std::{
    io::{self, Write},
    path::Path,
};

pub(super) fn run(
    options: &Options,
    workspace: &workspace::Workspace,
    path: &Path,
    whole_directory: bool,
) -> Result<(), NewError> {
    let original = crate::utf8_file::read(path).map_err(|e| NewError::Input(e.to_string()))?;
    let parsed = ParsedSpec::parse(&original);
    let snapshot = Snapshot::capture_supported(&parsed).map_err(NewError::Input)?;
    let path = fs::canonicalize(path).map_err(NewError::Workspace)?;
    let imported_name = path.file_stem();
    let package = options
        .pkgname
        .as_deref()
        .or_else(|| imported_name.and_then(|name| name.to_str()))
        .ok_or_else(|| NewError::Input("import name is not UTF-8; specify --pkgname".into()))?;
    let values = snapshot.document();
    let preview = options.stdout || options.diff;
    let mut area = workspace
        .new_development(
            &options.name,
            Some(package),
            preview,
            workspace::DevelopmentKind::Spec,
        )
        .map_err(NewError::Workspace)?;
    if path.file_stem().and_then(|name| name.to_str()) != Some(package) {
        crate::output_cli::stderr()
            .message(
                crate::output_cli::HumanLevel::Warn,
                Some(Path::new(&options.name)),
                format_args!(
                    "directory={package}; selected SPEC={}",
                    path.file_name().unwrap().to_string_lossy()
                ),
            )
            .map_err(NewError::Workspace)?;
    }
    let declared = values
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str);
    if declared != Some(package) {
        crate::output_cli::stderr()
            .message(
                crate::output_cli::HumanLevel::Warn,
                Some(Path::new(&options.name)),
                format_args!(
                    "directory={package}; SPEC Name={}",
                    declared.unwrap_or("not statically determined")
                ),
            )
            .map_err(NewError::Workspace)?;
    }
    let document = toml::to_string_pretty(values).map_err(|e| NewError::Input(e.to_string()))?;
    if preview {
        let output = if options.diff {
            file_output::target_diff(&area.manifest(), &document).map_err(NewError::Output)?
        } else {
            document
        };
        return io::stdout()
            .lock()
            .write_all(output.as_bytes())
            .map_err(|e| NewError::Output(file_output::OutputError::Stdout(e)));
    }
    if area.directory().try_exists().map_err(NewError::Workspace)? {
        return Err(NewError::Input(
            "SPEC import requires a new WORK; existing recipe and authoring files are retained"
                .into(),
        ));
    }
    import_directory(&path, &mut area, &original, &snapshot, values, whole_directory).map_err(|error| {
        NewError::Input(format!(
            "{error}; import was not published. Fix the cause and retry the same command; if WORK already exists, choose another WORK"
        ))
    })?;
    super::show_authoring(&options.name, &area.manifest())
}

fn import_directory(
    path: &Path,
    area: &mut workspace::Development,
    original: &str,
    snapshot: &Snapshot<'_>,
    values: &toml::Table,
    whole_directory: bool,
) -> Result<(), NewError> {
    let path = fs::canonicalize(path).map_err(NewError::Workspace)?;
    let parent = path.parent().expect("canonical SPEC has a parent");
    if whole_directory && area.directory().starts_with(parent) {
        return Err(NewError::Input(
            "import a dedicated package directory, not an ancestor of WORK".into(),
        ));
    }
    let filename = path.file_name().expect("canonical SPEC filename");
    let work = area.directory().parent().expect("WORK parent");
    fs::create_dir_all(work).map_err(NewError::Workspace)?;
    let temporary = tempfile::tempdir_in(work).map_err(NewError::Workspace)?;
    let recipe = temporary.path().join("recipe/SPECS").join(area.package());
    fs::create_dir_all(recipe.parent().expect("package parent")).map_err(NewError::Workspace)?;
    // Complete the copy before publishing any WORK; unsupported files leave no partial import.
    if whole_directory {
        crate::file_tree::copy(parent, &recipe).map_err(NewError::Workspace)?;
    } else {
        fs::create_dir(&recipe).map_err(NewError::Workspace)?;
        fs::copy(&path, recipe.join(path.file_name().expect("SPEC filename")))
            .map_err(NewError::Workspace)?;
    }
    if workspace::spec_in(&recipe, Some(area.package()))
        .map_err(NewError::Workspace)?
        .file_name()
        != Some(filename)
    {
        return Err(NewError::Input(
            "package binding selects a different SPEC; use --from-spec for an explicit file".into(),
        ));
    }
    let copied = recipe.join(path.file_name().expect("SPEC filename"));
    if crate::utf8_file::read(&copied).map_err(|e| NewError::Input(e.to_string()))? != original {
        return Err(NewError::Input("SPEC changed during import; retry".into()));
    }
    let target = area.package_directory().join(filename);
    draft::create(
        temporary.path(),
        &[draft::Input {
            path: &recipe.join(filename),
            destination: Some(&target),
            original,
            fields: snapshot.selection(),
            values,
        }],
    )
    .map_err(NewError::Input)?;
    workspace::save_baseline(temporary.path(), &recipe).map_err(NewError::Workspace)?;
    area.publish_prepared(temporary.path())
        .map_err(NewError::Workspace)
}
