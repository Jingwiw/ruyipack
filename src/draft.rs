// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Saved editable fields and the original bytes needed to apply them safely.

use std::{
    collections::HashSet,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use fs_err::{self as fs, OpenOptions};
use serde::{Deserialize, Serialize};

use crate::utf8_file;

pub(crate) struct Draft {
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
    original_sha256: String,
    fields: Vec<String>,
}

/// Read and verify `path`; persist `destination` only when preparing a relocated import.
pub(crate) struct Input<'a> {
    pub path: &'a Path,
    pub destination: Option<&'a Path>,
    pub original: &'a str,
    pub fields: &'a [String],
    pub values: &'a toml::Table,
}

pub(crate) fn create(dir: &Path, sources: &[Input<'_>]) -> Result<Vec<PathBuf>, String> {
    if sources.is_empty() {
        return Err("no sources selected for draft preparation".into());
    }
    let mut index = Index {
        version: 4,
        drafts: Vec::new(),
    };
    let mut contents = Vec::new();
    // Unique draft names also reject selecting the same source twice.
    let mut names = HashSet::new();
    // Validate and serialize every input before creating any state or draft files.
    for (position, input) in sources.iter().enumerate() {
        let Input {
            path: source,
            original,
            fields,
            values: table,
            ..
        } = input;
        let source = fs::canonicalize(source).map_err(|error| error.to_string())?;
        if !utf8_file::is_unchanged(&source, original)
            .map_err(|error| format!("cannot read source {}: {error}", source.display()))?
        {
            return Err(format!(
                "source {} changed since draft preparation; prepare fresh drafts",
                source.display()
            ));
        }
        let source = input.destination.map_or(source, Path::to_path_buf);
        if !source.is_absolute() {
            return Err("draft source binding must be absolute".into());
        }
        let draft = draft_name(&source)?;
        if !names.insert(draft.clone()) {
            return Err(format!(
                "duplicate source stem for {draft}; prepare these sources in separate draft directories"
            ));
        }
        let schema = format!("schema/{position}.json");
        let body = toml::to_string_pretty(table).map_err(|e| e.to_string())?;
        let document = format!(
            "#:tombi toml-version = \"v1.1.0\"\n#:schema .state/{schema}\n\n\
             # Edit the selected fields, then save this file.\n\
             # Preview with ruyipack edit --from DIR --diff before applying.\n\n{body}"
        );
        let schema_json =
            serde_json::to_vec_pretty(&crate::spec::document::schema::generate(table))
                .map_err(|error| format!("cannot serialize schema for {draft}: {error}"))?;
        contents.push((document, schema_json));
        index.drafts.push(Entry {
            source,
            original_sha256: utf8_file::sha256(original),
            fields: fields.to_vec(),
        });
    }
    let index_toml = toml::to_string_pretty(&index)
        .map_err(|error| format!("cannot serialize draft index: {error}"))?;
    ensure_absent(&dir.join(".state"))?;
    for entry in &index.drafts {
        ensure_absent(&dir.join(draft_name(&entry.source)?))?;
    }
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let dir = fs::canonicalize(dir).map_err(|error| error.to_string())?;
    let state = dir.join(".state");
    for path in [&state, &state.join("originals"), &state.join("schema")] {
        fs::create_dir(path).map_err(|error| error.to_string())?;
    }
    let mut drafts = Vec::new();
    for (position, ((entry, (document, schema)), input)) in
        index.drafts.iter().zip(contents).zip(sources).enumerate()
    {
        write_new(
            &state.join(format!(
                "originals/{position}-{}.spec",
                entry.original_sha256
            )),
            input.original.as_bytes(),
        )?;
        write_new(&state.join(format!("schema/{position}.json")), &schema)?;
        let path = dir.join(draft_name(&entry.source)?);
        write_new(&path, document.as_bytes())?;
        drafts.push(path);
    }
    // An interrupted preparation has no complete index and cannot be loaded.
    write_new(&state.join("index.toml"), index_toml.as_bytes())?;
    Ok(drafts)
}

pub(crate) fn load(dir: &Path) -> Result<Vec<Draft>, String> {
    let dir = fs::canonicalize(dir).map_err(|error| error.to_string())?;
    let state = dir.join(".state");
    for path in [&state, &state.join("originals"), &state.join("schema")] {
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if !metadata.file_type().is_dir() {
            return Err(format!(
                "draft state {} must be a directory, not a symlink",
                path.display()
            ));
        }
    }
    let index = read_index(&state.join("index.toml"))?;
    if index.drafts.is_empty() {
        return Err("draft index contains no sources".into());
    }
    let mut names = HashSet::new();
    let mut drafts = Vec::new();
    for (position, entry) in index.drafts.into_iter().enumerate() {
        if !entry.source.is_absolute() {
            return Err("draft index source paths must be absolute".into());
        }
        let name = draft_name(&entry.source)?;
        let path = dir.join(&name);
        if !names.insert(name) {
            return Err("draft index contains duplicate generated paths".into());
        }
        crate::source::validate_sha256(&entry.original_sha256)
            .map_err(|_| "draft index original_sha256 must be a SHA-256".to_owned())?;
        let original_path = state.join(format!(
            "originals/{position}-{}.spec",
            entry.original_sha256
        ));
        let original = read_text(&original_path)?;
        if utf8_file::sha256(&original) != entry.original_sha256 {
            return Err(format!(
                "saved original hash mismatch for {}",
                entry.source.display()
            ));
        }
        // Current source changes are reported per file by the candidate check.
        open_regular(&state.join(format!("schema/{position}.json")))?;
        open_regular(&path)?;
        drafts.push(Draft {
            source: entry.source,
            original,
            fields: entry.fields,
            path,
        });
    }
    Ok(drafts)
}

/// A destination must not replace the draft or its recovery inputs.
pub(crate) fn protect_output(output: &Path, draft: Option<&Path>) -> Result<(), String> {
    let Some(draft) = draft else {
        return Ok(());
    };
    let root = draft.parent().ok_or("draft has no parent directory")?;
    let target = crate::file_output::output_path(output).map_err(|e| e.to_string())?;
    if draft == target || target.starts_with(root.join(".state")) {
        return Err(format!(
            "{}: output must not replace editable input or recovery state",
            output.display()
        ));
    }
    #[cfg(unix)]
    if let Ok(target_metadata) = fs::metadata(&target) {
        use std::os::unix::fs::MetadataExt;
        let mut protected = vec![draft.to_path_buf(), root.join(".state/index.toml")];
        for directory in ["originals", "schema"] {
            for entry in
                fs::read_dir(root.join(".state").join(directory)).map_err(|e| e.to_string())?
            {
                protected.push(entry.map_err(|e| e.to_string())?.path());
            }
        }
        for path in protected {
            let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
            if metadata.dev() == target_metadata.dev() && metadata.ino() == target_metadata.ino() {
                return Err(format!(
                    "{}: output aliases a draft or its state",
                    output.display()
                ));
            }
        }
    }
    Ok(())
}

fn draft_name(source: &Path) -> Result<String, String> {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("source {} has no UTF-8 file stem", source.display()))?;
    Ok(format!("{stem}.toml"))
}

fn ensure_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
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
        use fs_err::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())
}

fn open_regular(path: &Path) -> Result<fs::File, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "draft file {} must be a regular file, not a symlink",
            path.display()
        ));
    }
    fs::File::open(path).map_err(|error| error.to_string())
}

pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    open_regular(path)?
        .read_to_string(&mut text)
        .map_err(|error| format!("cannot read draft file {}: {error}", path.display()))?;
    Ok(text)
}

fn read_index(path: &Path) -> Result<Index, String> {
    let index: Index = toml::from_str(&read_text(path)?)
        .map_err(|error| format!("invalid draft index {}: {error}", path.display()))?;
    if index.version != 4 {
        return Err(format!(
            "unsupported draft state version {} (expected 4)",
            index.version
        ));
    }
    Ok(index)
}

/// Read the selected TOML fields, not a second SPEC authority.
pub(crate) fn read_document_path(path: &Path) -> Result<toml::Table, String> {
    let text = read_text(path)?;
    toml::from_str(&text).map_err(|error: toml::de::Error| {
        if let Some(span) = error.span() {
            let prefix = &text[..span.start.min(text.len())];
            let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
            let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            format!("{}:{line}:{column}: {}", path.display(), error.message())
        } else {
            format!("{}: {}", path.display(), error.message())
        }
    })
}

pub(crate) fn save_document(path: &Path, document: &toml::Table) -> Result<(), String> {
    let body = toml::to_string_pretty(document).map_err(|e| e.to_string())?;
    let before = read_text(path)?;
    let directives = before
        .lines()
        .take_while(|line| line.starts_with("#:"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = if directives.is_empty() {
        body
    } else {
        format!("{directives}\n\n{body}")
    };
    crate::file_output::write_artifact(path, text.as_bytes()).map_err(|e| e.to_string())
}

/// Fill only digest fields already admitted by the draft's fixed mapping.
pub(crate) fn complete_digests(
    document: &mut toml::Table,
    downloads: &std::collections::BTreeMap<u32, crate::source::Download>,
) -> Result<(), String> {
    for (number, download) in downloads {
        let field = format!("sources.{number}.sha256");
        *crate::spec::document::table::lookup_mut(document, &field)
            .ok_or_else(|| format!("{field}: digest mapping unavailable"))? =
            toml::Value::String(download.sha256.clone());
    }
    Ok(())
}

/// Restore omitted keys, not explicit empty or invalid values supplied by the author.
pub(crate) fn complete_missing(document: &mut toml::Table, baseline: &toml::Table) {
    for (key, value) in baseline {
        match document.entry(key.clone()) {
            toml::map::Entry::Vacant(entry) => {
                entry.insert(value.clone());
            }
            toml::map::Entry::Occupied(mut entry) => {
                if let (Some(edited), Some(before)) =
                    (entry.get_mut().as_table_mut(), value.as_table())
                {
                    complete_missing(edited, before);
                }
            }
        }
    }
}

/// Update editable scope or advance a successfully applied baseline. The index
/// is published last: interrupted updates retain the previous recovery input.
pub(crate) fn update(
    path: &Path,
    fields: &[String],
    document: &toml::Table,
    applied: Option<&str>,
) -> Result<(), String> {
    let root = path.parent().ok_or("draft input has no parent")?;
    let state = root.join(".state");
    let index_path = state.join("index.toml");
    let mut index = read_index(&index_path)?;
    let (position, entry) = index
        .drafts
        .iter_mut()
        .enumerate()
        .find(|(_, entry)| draft_name(&entry.source).is_ok_and(|name| path == root.join(name)))
        .ok_or("draft input is not bound in its index")?;
    if let Some(original) = applied {
        if !utf8_file::is_unchanged(&entry.source, original).map_err(|e| e.to_string())? {
            return Err("applied source changed; retained draft baseline was not advanced".into());
        }
        entry.original_sha256 = utf8_file::sha256(original);
        // Never overwrite the indexed baseline before the index write succeeds.
        let baseline = state.join(format!(
            "originals/{position}-{}.spec",
            entry.original_sha256
        ));
        if !baseline.exists() {
            write_new(&baseline, original.as_bytes())?;
        } else if read_text(&baseline)? != original {
            return Err("saved baseline hash collision or corruption".into());
        }
    }
    entry.fields = fields.to_vec();
    let schema = serde_json::to_vec_pretty(&crate::spec::document::schema::generate(document))
        .map_err(|e| e.to_string())?;
    crate::file_output::write_artifact(&state.join(format!("schema/{position}.json")), &schema)
        .map_err(|e| e.to_string())?;
    save_document(path, document)?;
    let metadata = toml::to_string_pretty(&index).map_err(|e| e.to_string())?;
    crate::file_output::write_artifact(&index_path, metadata.as_bytes()).map_err(|e| e.to_string())
}

pub(crate) fn diff(path: &Path, original: &str, contents: &str) -> Result<String, String> {
    crate::file_output::diff_text(path, Some(original.as_bytes()), contents)
        .map_err(|e| e.to_string())
}

pub(crate) fn save_diff(dir: &Path, stem: &str, diff: &str) -> Result<PathBuf, String> {
    let path = dir.join(format!("{stem}.diff"));
    crate::file_output::write_artifact(&path, diff.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Candidates are derived from the input on every invocation, never trusted as a cache hit.
pub(crate) fn cache(dir: &Path, stem: &str, contents: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{stem}.candidate.spec"));
    crate::file_output::write_artifact(&path, contents.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(
        dir: &Path,
        sources: &[(PathBuf, String, Vec<String>, toml::Table)],
    ) -> Result<Vec<PathBuf>, String> {
        let sources = sources
            .iter()
            .map(|(p, s, f, t)| Input {
                path: p,
                destination: None,
                original: s,
                fields: f,
                values: t,
            })
            .collect::<Vec<_>>();
        super::create(dir, &sources)
    }

    #[test]
    fn relocated_draft_checks_input_bytes_and_records_the_final_source() {
        let root = tempfile::tempdir().unwrap();
        let (path, original, fields, values) = source(root.path(), "ed");
        let destination = root.path().join("final/recipe/ed.spec");
        let dir = root.path().join("authoring");
        let input = Input {
            path: &path,
            destination: Some(&destination),
            original: &original,
            fields: &fields,
            values: &values,
        };
        fs::write(&path, "changed").unwrap();
        assert!(super::create(&dir, std::slice::from_ref(&input)).is_err());
        assert!(!dir.exists());
        fs::write(&path, &original).unwrap();
        super::create(&dir, &[input]).unwrap();
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded[0].source, destination);
        assert_eq!(loaded[0].original, original);
    }

    fn source(dir: &Path, name: &str) -> (PathBuf, String, Vec<String>, toml::Table) {
        let path = dir.join(name);
        let original = "Name: demo\n".to_owned();
        fs::write(&path, &original).unwrap();
        let table = toml::from_str("name = 'demo'").unwrap();
        (path, original, vec!["name".into()], table)
    }

    #[test]
    fn reading_drafts_reports_io_context_and_rejects_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.toml");
        let error = read_text(&path).unwrap_err();
        assert!(
            error.contains(path.to_str().unwrap()) && error.contains("metadata"),
            "{error}"
        );
        assert!(ensure_absent(&path).is_ok());
        write_new(&path, b"after").unwrap();
        let error = write_new(&path, b"overwrite").unwrap_err();
        assert!(
            error.contains(path.to_str().unwrap()) && error.contains("open file"),
            "{error}"
        );
        assert_eq!(read_text(&path).unwrap(), "after");
        #[cfg(unix)]
        {
            fs::remove_file(&path).unwrap();
            let other = dir.path().join("other");
            fs::write(&other, "external").unwrap();
            std::os::unix::fs::symlink(&other, &path).unwrap();
            assert!(read_text(&path).unwrap_err().contains("not a symlink"));
        }
    }

    #[test]
    fn roundtrip_keeps_business_fields_separate_from_originals() {
        let temp = tempfile::tempdir().unwrap();
        let input = source(temp.path(), "demo.spec");
        let dir = temp.path().join("drafts");
        let prepared = create(&dir, std::slice::from_ref(&input)).unwrap();
        assert_eq!(prepared.len(), 1);
        let text = fs::read_to_string(&prepared[0]).unwrap();
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
        assert_eq!(loaded[0].path, prepared[0]);
        assert_eq!(fs::read_to_string(&input.0).unwrap(), input.1);
        assert_eq!(
            fs::read_to_string(dir.join(format!(
                ".state/originals/0-{}.spec",
                utf8_file::sha256(&input.1)
            )))
            .unwrap(),
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
        fs::write(
            dir.join(format!(
                ".state/originals/0-{}.spec",
                utf8_file::sha256(&input.1)
            )),
            "Name: changed\n",
        )
        .unwrap();
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
                dir.join(format!(
                    ".state/originals/{position}-{}.spec",
                    utf8_file::sha256(&inputs[position].1)
                ))
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
        let path = dir.join(".state/index.toml");
        let original: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let mut invalid = Vec::new();
        let mut value = original.clone();
        value["version"] = toml::Value::Integer(999);
        invalid.push(value);
        let mut value = original.clone();
        value
            .as_table_mut()
            .unwrap()
            .insert("unexpected".into(), toml::Value::Boolean(true));
        invalid.push(value);
        let mut value = original.clone();
        value["drafts"][0]["source"] = toml::Value::String("demo.spec".into());
        invalid.push(value);
        let mut value = original.clone();
        value["drafts"] = toml::Value::Array(Vec::new());
        invalid.push(value);
        let mut value = original.clone();
        value["drafts"] = toml::Value::Array(vec![
            original["drafts"][0].clone(),
            original["drafts"][0].clone(),
        ]);
        invalid.push(value);
        let mut value = original;
        value["drafts"][0]
            .as_table_mut()
            .unwrap()
            .insert("unexpected".into(), toml::Value::Boolean(true));
        invalid.push(value);
        for value in invalid {
            fs::write(&path, toml::to_string_pretty(&value).unwrap()).unwrap();
            assert!(load(&dir).is_err(), "{value}");
        }
    }

    #[test]
    fn preparation_refuses_collisions_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let input = source(temp.path(), "demo.spec");
        let duplicate = source(temp.path(), "demo.other");
        let dir = temp.path().join("drafts");
        assert!(create(&dir, &[input.clone(), input.clone()]).is_err());
        assert!(!dir.exists());
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
        let index = fs::read(fresh.join(".state/index.toml")).unwrap();
        assert!(create(&fresh, &[input]).is_err());
        assert_eq!(fs::read(fresh.join(".state/index.toml")).unwrap(), index);
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
        let original = dir.join(format!(
            ".state/originals/0-{}.spec",
            utf8_file::sha256(&input.1)
        ));
        fs::remove_file(&original).unwrap();
        symlink(&replacement.0, &original).unwrap();
        assert!(load(&dir).err().unwrap().contains("not a symlink"));
    }
}
