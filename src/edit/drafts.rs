// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Editable business fields and the original bytes needed to apply them safely.

use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) struct Draft {
    pub source: PathBuf,
    pub original: String,
    pub fields: Vec<String>,
    pub path: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    drafts: Vec<Entry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    source: PathBuf,
    original: String,
    original_sha256: String,
    fields: Vec<String>,
    draft: String,
    schema: String,
}

pub(super) fn create(
    dir: &Path,
    sources: &[(PathBuf, String, Vec<String>, toml::Table)],
) -> Result<Vec<Draft>, String> {
    if sources.is_empty() {
        return Err("no sources selected for draft preparation".into());
    }
    let mut index = Index {
        version: 1,
        drafts: Vec::new(),
    };
    let mut contents = Vec::new();
    let mut source_paths = HashSet::new();
    let mut names = HashSet::new();
    // Validate and serialize every input before creating any state or draft files.
    for (position, (source, original, fields, table)) in sources.iter().enumerate() {
        let source = fs::canonicalize(source)
            .map_err(|error| format!("cannot resolve source {}: {error}", source.display()))?;
        if !source_paths.insert(source.clone()) {
            return Err(format!(
                "source {} was selected more than once",
                source.display()
            ));
        }
        check_source(&source, original)?;
        let draft = draft_name(&source)?;
        if !names.insert(draft.clone()) {
            return Err(format!(
                "duplicate source stem for {draft}; prepare these sources in separate draft directories"
            ));
        }
        let schema = format!("schema/{position}.json");
        let body = toml::to_string_pretty(table)
            .map_err(|error| format!("cannot serialize {draft}: {error}"))?;
        let document = format!(
            "#:tombi toml-version = \"v1.1.0\"\n#:schema .state/{schema}\n\n\
             # Edit the selected fields, then save this file.\n\
             # Preview with ruyipack edit --from DIR --diff before applying.\n\n{body}"
        );
        let schema_json = serde_json::to_vec_pretty(&super::fields::schema(table))
            .map_err(|error| format!("cannot serialize schema for {draft}: {error}"))?;
        contents.push((document, schema_json));
        index.drafts.push(Entry {
            source,
            original: format!("originals/{position}.spec"),
            original_sha256: digest(original),
            fields: fields.clone(),
            draft,
            schema,
        });
    }
    let index_json = serde_json::to_vec_pretty(&index)
        .map_err(|error| format!("cannot serialize draft index: {error}"))?;
    ensure_absent(&dir.join(".state"))?;
    for entry in &index.drafts {
        ensure_absent(&dir.join(&entry.draft))?;
    }
    fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create draft directory {}: {error}", dir.display()))?;
    let dir = fs::canonicalize(dir)
        .map_err(|error| format!("cannot resolve draft directory {}: {error}", dir.display()))?;
    let state = dir.join(".state");
    for path in [&state, &state.join("originals"), &state.join("schema")] {
        fs::create_dir(path)
            .map_err(|error| format!("cannot create draft state {}: {error}", path.display()))?;
    }
    let mut drafts = Vec::new();
    for ((entry, (document, schema)), (_, original, _, _)) in
        index.drafts.iter().zip(contents).zip(sources)
    {
        write_new(&state.join(&entry.original), original.as_bytes())?;
        write_new(&state.join(&entry.schema), &schema)?;
        let path = dir.join(&entry.draft);
        write_new(&path, document.as_bytes())?;
        drafts.push(Draft {
            source: entry.source.clone(),
            original: original.clone(),
            fields: entry.fields.clone(),
            path,
        });
    }
    // An interrupted preparation has no complete index and cannot be loaded.
    write_new(&state.join("index.json"), &index_json)?;
    Ok(drafts)
}

pub(super) fn load(dir: &Path) -> Result<Vec<Draft>, String> {
    let dir = fs::canonicalize(dir)
        .map_err(|error| format!("cannot resolve draft directory {}: {error}", dir.display()))?;
    let state = dir.join(".state");
    for path in [&state, &state.join("originals"), &state.join("schema")] {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("cannot read draft state {}: {error}", path.display()))?;
        if !metadata.file_type().is_dir() {
            return Err(format!(
                "draft state {} must be a directory, not a symlink",
                path.display()
            ));
        }
    }
    let index_path = state.join("index.json");
    let index: Index = serde_json::from_slice(&read_regular(&index_path)?)
        .map_err(|error| format!("invalid draft index {}: {error}", index_path.display()))?;
    if index.version != 1 {
        return Err(format!(
            "unsupported draft state version {} (expected 1)",
            index.version
        ));
    }
    if index.drafts.is_empty() {
        return Err("draft index contains no sources".into());
    }
    let mut source_paths = HashSet::new();
    let mut names = HashSet::new();
    let mut drafts = Vec::new();
    for (position, entry) in index.drafts.into_iter().enumerate() {
        if !entry.source.is_absolute() || !source_paths.insert(entry.source.clone()) {
            return Err("draft index source paths must be absolute and unique".into());
        }
        let expected_name = draft_name(&entry.source)?;
        if entry.draft != expected_name
            || !names.insert(entry.draft.clone())
            || entry.original != format!("originals/{position}.spec")
            || entry.schema != format!("schema/{position}.json")
        {
            return Err("draft index contains invalid or duplicate generated paths".into());
        }
        let original_path = state.join(&entry.original);
        let original = String::from_utf8(read_regular(&original_path)?).map_err(|error| {
            format!(
                "invalid saved original {}: {error}",
                original_path.display()
            )
        })?;
        if digest(&original) != entry.original_sha256 {
            return Err(format!(
                "saved original hash mismatch for {}",
                entry.source.display()
            ));
        }
        // Current source changes are reported per file by the candidate check.
        read_regular(&state.join(&entry.schema))?;
        let path = dir.join(&entry.draft);
        read_regular(&path)?;
        drafts.push(Draft {
            source: entry.source,
            original,
            fields: entry.fields,
            path,
        });
    }
    Ok(drafts)
}

fn check_source(source: &Path, original: &str) -> Result<(), String> {
    let current_path = fs::canonicalize(source)
        .map_err(|error| format!("cannot resolve source {}: {error}", source.display()))?;
    if current_path != source {
        return Err(format!(
            "source identity changed for {}; prepare fresh drafts",
            source.display()
        ));
    }
    let current = fs::read(source)
        .map_err(|error| format!("cannot read source {}: {error}", source.display()))?;
    if current != original.as_bytes() {
        return Err(format!(
            "source {} changed since draft preparation; prepare fresh drafts",
            source.display()
        ));
    }
    Ok(())
}

fn draft_name(source: &Path) -> Result<String, String> {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("source {} has no UTF-8 file stem", source.display()))?;
    let name = format!("{stem}.toml");
    let mut components = Path::new(&name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(format!(
            "source {} has an invalid draft file stem",
            source.display()
        ));
    }
    Ok(name)
}

fn digest(original: &str) -> String {
    format!("{:x}", Sha256::digest(original.as_bytes()))
}

fn ensure_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
        Ok(_) => Err(format!(
            "draft path {} already exists; use a new draft directory",
            path.display()
        )),
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot create draft file {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write draft file {}: {error}", path.display()))
}

fn read_regular(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect draft file {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "draft file {} must be a regular file, not a symlink",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| format!("cannot read draft file {}: {error}", path.display()))
}
