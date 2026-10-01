// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Publication conflicts, source changes, and partial failures.

use super::*;
use std::assert_matches;

fn no_prompt(_: &Path) -> Result<ConflictAction, OutputError> {
    panic!("this operation must not ask for a conflict selection")
}

fn source(directory: &Path, name: &str, contents: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, contents).unwrap();
    path.canonicalize().unwrap()
}

#[test]
fn stage_artifacts_publish_binary_bytes_and_refuse_nonregular_targets() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = directory.path().join("candidate.bin");
    write_artifact(&artifact, b"first\x00\xff").unwrap();
    assert_eq!(fs::read(&artifact).unwrap(), b"first\x00\xff");
    write_artifact(&artifact, b"replacement\x00").unwrap();
    assert_eq!(fs::read(&artifact).unwrap(), b"replacement\x00");
    assert_eq!(
        write_artifact(directory.path(), b"no").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn stage_artifacts_refuse_symlink_and_hardlink_aliases_without_changing_bytes() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let original = source(directory.path(), "original", "pristine\n");
    let symlink_path = directory.path().join("symlink");
    symlink(&original, &symlink_path).unwrap();
    assert_eq!(
        write_artifact(&symlink_path, b"candidate")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(
        fs::symlink_metadata(&symlink_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let hardlink = directory.path().join("hardlink");
    fs::hard_link(&original, &hardlink).unwrap();
    for path in [&original, &hardlink] {
        assert_eq!(
            write_artifact(path, b"candidate").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "pristine\n");
    }
}

#[test]
fn diff_headers_preserve_data_paths_and_terminate_spaced_filenames() {
    let path = Path::new("dir with spaces/pkg.spec");
    let diff = diff_text(path, Some(b"old\n"), "new\n").unwrap();
    assert!(diff.starts_with("--- dir with spaces/pkg.spec\t\n+++ dir with spaces/pkg.spec\t\n"));
    let diff = diff_text(path, None, "new\n").unwrap();
    assert!(diff.starts_with("--- /dev/null\n+++ dir with spaces/pkg.spec\t\n"));
    for invalid in ["pkg\t.spec", "pkg\r.spec", "pkg\n.spec"] {
        assert_matches!(diff_text(Path::new(invalid), Some(b"old\n"), "new\n"),
            Err(OutputError::DiffPath(actual)) if actual == Path::new(invalid));
    }
    assert_matches!(diff_text(path, Some(b"old\xff\n"), "new\n"),
        Err(OutputError::DiffEncoding { path: actual, .. }) if actual == path);
}

#[cfg(unix)]
#[test]
fn diff_headers_reject_non_utf8_names_instead_of_replacing_their_identity() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let path = PathBuf::from(OsString::from_vec(b"pkg\xff.spec".to_vec()));
    assert_matches!(diff_text(&path, Some(b"old\n"), "new\n"),
        Err(OutputError::DiffPath(actual)) if actual == path);
}

#[test]
fn copy_selection_is_unreachable_for_directories_or_nameless_targets() {
    let directory = tempfile::tempdir().unwrap();
    let input = source(directory.path(), "input.spec", "original\n");
    let files = [EditFile {
        source_path: &input,
        original: "original\n",
        contents: "candidate\n",
    }];
    for target in [directory.path(), Path::new(""), Path::new("/")] {
        assert!(publish(&mut io::sink(), target, "candidate\n", no_prompt).is_err());
        assert!(run_edits(&mut io::sink(), &files, Some(target), no_prompt).is_err());
    }
    assert_eq!(fs::read_to_string(input).unwrap(), "original\n");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn copy_path_preserves_non_utf8_names() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let path = PathBuf::from(OsString::from_vec(b"output\xff.spec".to_vec()));
    for (number, expected) in [
        (0, b"output\xff.spec.new".as_slice()),
        (1, b"output\xff.spec.new.1".as_slice()),
    ] {
        assert_eq!(
            copy_path(&path, number).into_os_string().into_vec(),
            expected
        );
    }
}

#[test]
fn copy_selection_preserves_extensions_and_existing_sidecars() {
    for edit in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let input = source(directory.path(), "input.spec", "original\n");
        let target = directory.path().join("output.review.spec");
        let occupied = directory.path().join("output.review.spec.new");
        let copy = directory.path().join("output.review.spec.new.1");
        fs::write(&target, "other\n").unwrap();
        fs::write(&occupied, "keep\n").unwrap();
        if edit {
            let outcomes = run_edits(
                &mut io::sink(),
                &[EditFile {
                    source_path: &input,
                    original: "original\n",
                    contents: "candidate\n",
                }],
                Some(&target),
                |_| Ok(ConflictAction::Copy),
            )
            .unwrap();
            assert_eq!(
                outcomes,
                [EditOutcome::Written(copy.canonicalize().unwrap())]
            );
        } else {
            publish(&mut io::sink(), &target, "candidate\n", |_| {
                Ok(ConflictAction::Copy)
            })
            .unwrap();
        }
        for (path, expected) in [
            (&input, "original\n"),
            (&target, "other\n"),
            (&occupied, "keep\n"),
            (&copy, "candidate\n"),
        ] {
            assert_eq!(fs::read_to_string(path).unwrap(), expected);
        }
    }
}

#[test]
fn edit_selection_cannot_authorize_a_changed_source_or_destination() {
    for change_source in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let input = source(directory.path(), "input.spec", "original\n");
        let target = source(directory.path(), "output.spec", "other\n");
        let changed = if change_source { &input } else { &target };
        let result = run_edits(
            &mut io::sink(),
            &[EditFile {
                source_path: &input,
                original: "original\n",
                contents: "candidate\n",
            }],
            Some(&target),
            |path| {
                assert_eq!(path, target);
                fs::write(changed, "external change\n").unwrap();
                Ok(ConflictAction::Overwrite)
            },
        );
        if change_source {
            assert_matches!(result, Err(OutputError::SourceChanged(path)) if path == input);
            assert_eq!(fs::read_to_string(&target).unwrap(), "other\n");
        } else {
            assert_matches!(result, Err(OutputError::Changed(path)) if path == target);
            assert_eq!(fs::read_to_string(&input).unwrap(), "original\n");
        }
        assert_eq!(fs::read_to_string(changed).unwrap(), "external change\n");
    }
}

#[test]
fn edit_diff_returns_to_selection_and_cancellation_does_not_publish() {
    let directory = tempfile::tempdir().unwrap();
    let input = source(directory.path(), "input.spec", "original\n");
    let target = source(directory.path(), "output.spec", "other\n");
    let mut selections = 0;
    let mut diff = Vec::new();
    let result = run_edits(
        &mut diff,
        &[EditFile {
            source_path: &input,
            original: "original\n",
            contents: "candidate\n",
        }],
        Some(&target),
        |_| {
            selections += 1;
            if selections == 1 {
                Ok(ConflictAction::Diff)
            } else {
                Err(OutputError::Selection(Box::new(io::Error::other(
                    "cancelled",
                ))))
            }
        },
    );
    let error = result.unwrap_err();
    assert_eq!(selections, 2);
    assert_eq!(
        String::from_utf8(diff).unwrap(),
        format!(
            "--- {0}\t\n+++ {0}\t\n@@ -1 +1 @@\n-other\n+candidate\n",
            target.display()
        )
    );
    assert_eq!(
        std::error::Error::source(&error).unwrap().to_string(),
        "cancelled"
    );
    assert_eq!(fs::read_to_string(input).unwrap(), "original\n");
    assert_eq!(fs::read_to_string(target).unwrap(), "other\n");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn generated_diff_returns_to_selection_and_rechecks_the_target() {
    let directory = tempfile::tempdir().unwrap();
    let target = source(directory.path(), "output.spec", "other\n");
    let mut selections = 0;
    let mut diff = Vec::new();
    let result = publish(&mut diff, &target, "candidate\n", |path| {
        selections += 1;
        if selections == 1 {
            Ok(ConflictAction::Diff)
        } else {
            fs::write(path, "external change\n").unwrap();
            Ok(ConflictAction::Overwrite)
        }
    });
    assert_matches!(result, Err(OutputError::Changed(path)) if path == target);
    assert_eq!(selections, 2);
    assert_eq!(
        String::from_utf8(diff).unwrap(),
        format!(
            "--- {0}\t\n+++ {0}\t\n@@ -1 +1 @@\n-other\n+candidate\n",
            target.display()
        )
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "external change\n");
}

#[test]
fn all_sources_are_checked_before_force_writes_any_member() {
    let directory = tempfile::tempdir().unwrap();
    let first = source(directory.path(), "first.spec", "first\n");
    let second = source(directory.path(), "second.spec", "changed externally\n");
    let files = [
        EditFile {
            source_path: &first,
            original: "first\n",

            contents: "edited first\n",
        },
        EditFile {
            source_path: &second,
            original: "second\n",

            contents: "edited second\n",
        },
    ];
    std::assert_matches!(
        run_edits(&mut io::sink(), &files, None, |_| Ok(ConflictAction::Overwrite)),
        Err(OutputError::SourceChanged(path)) if path == second
    );
    assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
    assert_eq!(fs::read_to_string(second).unwrap(), "changed externally\n");
}

#[cfg(unix)]
#[test]
fn repeated_source_checks_reject_a_same_bytes_symlink_replacement() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let input = source(directory.path(), "input.spec", "original\n");
    let replacement = source(directory.path(), "replacement.spec", "original\n");
    let files = [EditFile {
        source_path: &input,
        original: "original\n",

        contents: "edited\n",
    }];
    check_sources(&files, &[]).unwrap();
    fs::remove_file(&input).unwrap();
    symlink(&replacement, &input).unwrap();
    assert_matches!(
        check_sources(&files, &[]),
        Err(OutputError::SourceChanged(path)) if path == input
    );
}

#[test]
fn successful_overwrites_do_not_invalidate_the_remaining_batch() {
    let directory = tempfile::tempdir().unwrap();
    let first = source(directory.path(), "first.spec", "first\n");
    let second = source(directory.path(), "second.spec", "second\n");
    let files = [
        EditFile {
            source_path: &first,
            original: "first\n",

            contents: "edited first\n",
        },
        EditFile {
            source_path: &second,
            original: "second\n",

            contents: "edited second\n",
        },
    ];
    assert_eq!(
        run_edits(&mut io::sink(), &files, None, |_| Ok(
            ConflictAction::Overwrite
        ))
        .unwrap(),
        vec![
            EditOutcome::Written(first.clone()),
            EditOutcome::Written(second.clone())
        ]
    );
    assert_eq!(fs::read_to_string(first).unwrap(), "edited first\n");
    assert_eq!(fs::read_to_string(second).unwrap(), "edited second\n");
}

#[cfg(unix)]
#[test]
fn hardlinked_sources_and_targets_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let first = source(directory.path(), "first.spec", "first\n");
    let alias = directory.path().join("alias.spec");
    fs::hard_link(&first, &alias).unwrap();
    let alias = alias.canonicalize().unwrap();
    let own_alias = [EditFile {
        source_path: &first,
        original: "first\n",

        contents: "edited\n",
    }];
    assert_matches!(
        run_edits(&mut io::sink(), &own_alias, Some(&alias), |_| Ok(
            ConflictAction::Overwrite
        )),
        Err(OutputError::EditLayout(_))
    );
    let duplicate_sources = [
        EditFile {
            source_path: &first,
            original: "first\n",

            contents: "edited\n",
        },
        EditFile {
            source_path: &alias,
            original: "first\n",

            contents: "edited\n",
        },
    ];
    assert_matches!(
        run_edits(&mut io::sink(), &duplicate_sources, None, |_| Ok(
            ConflictAction::Overwrite
        )),
        Err(OutputError::EditLayout(_))
    );
    assert_eq!(fs::read_to_string(first).unwrap(), "first\n");
}

#[cfg(unix)]
#[test]
fn edits_preserve_access_permissions_on_existing_and_new_targets() {
    let directory = tempfile::tempdir().unwrap();
    let input = source(directory.path(), "input.spec", "original\n");
    fs::set_permissions(&input, fs::Permissions::from_mode(0o640)).unwrap();
    let target = directory.path().canonicalize().unwrap().join("new.spec");
    run_edits(
        &mut io::sink(),
        &[EditFile {
            source_path: &input,
            original: "original\n",

            contents: "edited\n",
        }],
        Some(&target),
        |_| Ok(ConflictAction::Overwrite),
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    run_edits(
        &mut io::sink(),
        &[EditFile {
            source_path: &input,
            original: "original\n",

            contents: "edited\n",
        }],
        None,
        |_| Ok(ConflictAction::Overwrite),
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&input).unwrap().permissions().mode() & 0o7777,
        0o640
    );
}
#[test]
fn unchanged_edits_do_not_replace_files() {
    let directory = tempfile::tempdir().unwrap();
    let input = source(directory.path(), "unchanged.spec", "same\n");
    let before = fs::metadata(&input).unwrap();
    let files = [EditFile {
        source_path: &input,
        original: "same\n",

        contents: "same\n",
    }];
    assert_eq!(
        run_edits(&mut io::sink(), &files, None, |_| Ok(
            ConflictAction::Overwrite
        ))
        .unwrap(),
        vec![EditOutcome::Unchanged(input.clone())]
    );
    let after = fs::metadata(&input).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    #[cfg(unix)]
    assert_eq!(before.ino(), after.ino());
    assert_eq!(fs::read_to_string(&input).unwrap(), "same\n");
}

#[cfg(unix)]
#[test]
fn a_later_write_failure_retains_exact_written_paths() {
    let directory = tempfile::tempdir().unwrap();
    let first = source(directory.path(), "first, with spaces.spec", "first\n");
    let locked = directory.path().join("locked");
    fs::create_dir(&locked).unwrap();
    let second = source(&locked, "second.spec", "second\n");
    let files = [
        EditFile {
            source_path: &first,
            original: "first\n",

            contents: "changed first\n",
        },
        EditFile {
            source_path: &second,
            original: "second\n",

            contents: "changed second\n",
        },
    ];
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let result = run_edits(&mut io::sink(), &files, None, |_| {
        Ok(ConflictAction::Overwrite)
    });
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let Err(OutputError::Partial { written, source }) = result else {
        panic!("expected a partial permission failure: {result:?}")
    };
    assert_eq!(written, vec![first.clone()]);
    assert_matches!(*source, OutputError::Write { .. });
    assert_eq!(fs::read_to_string(first).unwrap(), "changed first\n");
    assert_eq!(fs::read_to_string(second).unwrap(), "second\n");
}
