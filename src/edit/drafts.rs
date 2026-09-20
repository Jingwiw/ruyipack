// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Saved editable fields and the original bytes needed to apply them safely.

use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::utf8_file;

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
        if !utf8_file::is_unchanged(&source, original)
            .map_err(|error| format!("cannot read source {}: {error}", source.display()))?
        {
            return Err(format!(
                "source {} changed since draft preparation; prepare fresh drafts",
                source.display()
            ));
        }
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
            original_sha256: utf8_file::digest(original),
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
        if utf8_file::digest(&original) != entry.original_sha256 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn source(dir: &Path, name: &str) -> (PathBuf, String, Vec<String>, toml::Table) {
        let path = dir.join(name);
        let original = "Name: demo\n".to_owned();
        fs::write(&path, &original).unwrap();
        let table = toml::from_str("name = 'demo'").unwrap();
        (path, original, vec!["name".into()], table)
    }

    #[test]
    fn roundtrip_keeps_business_fields_separate_from_originals() {
        let temp = tempfile::tempdir().unwrap();
        let input = source(temp.path(), "demo.spec");
        let dir = temp.path().join("drafts");
        let prepared = create(&dir, std::slice::from_ref(&input)).unwrap();
        assert_eq!(prepared.len(), 1);
        let text = fs::read_to_string(&prepared[0].path).unwrap();
        assert!(
            text.starts_with(
                "#:tombi toml-version = \"v1.1.0\"\n#:schema .state/schema/0.json\n\n"
            )
        );
        assert_eq!(toml::from_str::<toml::Table>(&text).unwrap(), input.3);
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded[0].source, fs::canonicalize(&input.0).unwrap());
        assert_eq!(loaded[0].original, input.1);
        assert_eq!(loaded[0].fields, input.2);
        assert_eq!(loaded[0].path, prepared[0].path);
        assert_eq!(fs::read_to_string(&input.0).unwrap(), input.1);
        assert_eq!(
            fs::read_to_string(dir.join(".state/originals/0.spec")).unwrap(),
            input.1
        );
    }

    #[test]
    fn source_checks_are_deferred_but_saved_originals_are_verified() {
        let temp = tempfile::tempdir().unwrap();
        let input = source(temp.path(), "demo.spec");
        let dir = temp.path().join("drafts");
        create(&dir, std::slice::from_ref(&input)).unwrap();
        fs::write(&input.0, "Name: changed\n").unwrap();
        assert_eq!(load(&dir).unwrap()[0].original, input.1);
        fs::write(&input.0, &input.1).unwrap();
        fs::write(dir.join(".state/originals/0.spec"), "Name: changed\n").unwrap();
        assert!(load(&dir).err().unwrap().contains("hash mismatch"));
    }

    #[test]
    fn all_sources_are_checked_before_preparation_and_keep_their_order() {
        let temp = tempfile::tempdir().unwrap();
        let first = source(temp.path(), "first.spec");
        let second = source(temp.path(), "second.spec");
        let inputs = [first, second];
        let dir = temp.path().join("drafts");
        fs::write(&inputs[1].0, "Name: stale\n").unwrap();
        assert!(create(&dir, &inputs).is_err());
        assert!(!dir.exists());
        fs::write(&inputs[1].0, &inputs[1].1).unwrap();
        create(&dir, &inputs).unwrap();
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.len(), 2);
        for (position, draft) in loaded.iter().enumerate() {
            assert_eq!(draft.source, fs::canonicalize(&inputs[position].0).unwrap());
            assert_eq!(draft.original, inputs[position].1);
            assert!(
                dir.join(format!(".state/originals/{position}.spec"))
                    .is_file()
            );
            assert!(dir.join(format!(".state/schema/{position}.json")).is_file());
        }
    }

    #[test]
    fn invalid_metadata_is_rejected_before_loading_drafts() {
        let temp = tempfile::tempdir().unwrap();
        let input = source(temp.path(), "demo.spec");
        let dir = temp.path().join("drafts");
        create(&dir, &[input]).unwrap();
        let path = dir.join(".state/index.json");
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut invalid = Vec::new();
        let mut value = original.clone();
        value["version"] = 2.into();
        invalid.push(value);
        let mut value = original.clone();
        value["unexpected"] = true.into();
        invalid.push(value);
        for field in ["original", "schema", "draft"] {
            let mut value = original.clone();
            value["drafts"][0][field] = "../outside".into();
            invalid.push(value);
        }
        let mut value = original.clone();
        value["drafts"][0]["unexpected"] = true.into();
        invalid.push(value);
        for value in invalid {
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(load(&dir).is_err(), "{value}");
        }
    }

    #[test]
    fn preparation_refuses_collisions_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let input = source(temp.path(), "demo.spec");
        let duplicate = source(temp.path(), "demo.other");
        let dir = temp.path().join("drafts");
        assert!(
            create(&dir, &[input.clone(), duplicate])
                .err()
                .unwrap()
                .contains("duplicate source stem")
        );
        assert!(!dir.exists());
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("demo.toml"), "keep me").unwrap();
        assert!(create(&dir, std::slice::from_ref(&input)).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("demo.toml")).unwrap(),
            "keep me"
        );
        assert!(!dir.join(".state").exists());
        let fresh = temp.path().join("fresh");
        create(&fresh, std::slice::from_ref(&input)).unwrap();
        let index = fs::read(fresh.join(".state/index.json")).unwrap();
        assert!(create(&fresh, &[input]).is_err());
        assert_eq!(fs::read(fresh.join(".state/index.json")).unwrap(), index);
    }

    #[cfg(unix)]
    #[test]
    fn state_symlink_replacements_are_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let input = source(temp.path(), "demo.spec");
        let replacement = source(temp.path(), "replacement.spec");
        let dir = temp.path().join("drafts");
        create(&dir, std::slice::from_ref(&input)).unwrap();
        fs::remove_file(&input.0).unwrap();
        symlink(&replacement.0, &input.0).unwrap();
        assert_eq!(
            load(&dir).unwrap()[0].source,
            fs::canonicalize(temp.path()).unwrap().join("demo.spec")
        );
        fs::remove_file(&input.0).unwrap();
        fs::write(&input.0, &input.1).unwrap();
        let original = dir.join(".state/originals/0.spec");
        fs::remove_file(&original).unwrap();
        symlink(&replacement.0, &original).unwrap();
        assert!(load(&dir).err().unwrap().contains("not a symlink"));
    }
}
