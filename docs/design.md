<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# Ownership and validation boundaries

RuyiPack calculates candidates before publishing them. It does not need a package
workspace database, execution framework, or a persistent package-wide mode to do so.

## Authority belongs to the operation

- `gen` reads the selected manifest and embedded profile/build defaults. The SPEC
  is derived output. Existing output is only consulted for publication conflicts
  or a requested diff, never merged back into the manifest.
- `edit` reads the selected SPEC and replaces supported field ranges. It does not
  read or update a neighboring manifest, even if the filenames match.
- Drafts are proposals tied to original SPEC bytes, not another authority.
  Source changes invalidate the proposal; `--force` does not bypass this check.
- Reports describe particular input/candidate bytes. They are evidence, not
  editable package state. A successful check does not authorize publication.

Users who edit a generated SPEC must reconcile the manifest themselves before
regenerating. No filename heuristic chooses an authority or synchronizes files.

## Current producer and consumer boundaries

| Producer | Value | Consumer |
| --- | --- | --- |
| `render` | Generated contents, static report, selected build contract | `generate` previews, reports, or publishes |
| `spec::document::Snapshot` | Original bytes, selected fields and replacement ranges | `edit::candidate` calculates and validates an edit |
| `edit::candidate` | Candidate contents, static report, review triggers | `edit` checks, previews, or publishes |
| `file_output` | Written/unchanged/skipped paths, typed partial failures | `edit` formats results and retains recovery information |
| `check` | Selected static rule results and parser diagnostics | `check`, `gen`, and `edit` reports |

Read `generate.rs` and `edit.rs` for orchestration, then `render.rs`,
`edit/candidate.rs`, and `spec/document.rs` for calculation. `file_output.rs` owns
conflict handling and publication. CLI/editor interactions stay outside candidate
calculation. These are private modules, not a promised Rust library API.

## Facts, decisions, and evidence

`spec/` adapts parser syntax and source locations into the limited views the tool
uses. Syntax is not a macro-expanded RPM value. Unsupported or ambiguous selected
fields must not be guessed. Packaging policy remains in the rendering/checking
code and embedded profile contracts, not in an upstream parser's data model.

JSON identifies exact inputs and the actual parser. `gen --check` additionally
identifies its manifest and selected profile/build contract; `edit --check`
distinguishes original and candidate hashes. Profile hashes cover embedded TOML,
not every validation rule or a target environment. Paths identify sources or
intended destinations; hashes identify the bytes actually checked.

The `spec-static` stage only proves selected static checks. Native Source-numbering
checks in `scripts/check-native-sources` are separate evidence, and neither proves
a package builds. Build evidence needs the actual source, environment, command,
and resulting artifacts; absent evidence is not success. No universal success
flag spans these stages.

## Publication and recovery

All batch candidates are validated before publication. Publication is atomic per
file, not per batch. Typed results preserve exact paths rather than a comma-joined
list. A later I/O failure retains already-written paths; callers must inspect
those files before retrying. Human notices are emitted after a successful batch,
and a notice-write failure cannot roll back publication.

Source identity checks detect common concurrent edits, not arbitrary races.
Draft hashes establish consistency, not trust in an imported draft. See the
README for overwrite, permissions, and durability limits.

## Regression contracts

- `edit::candidate` tests repeatability and exact preservation outside the selected
  version range; `tests/edit_selected.rs` covers unsupported unrelated syntax.
- `file_output` tests no-op byte/inode/mtime preservation, stale-source rejection,
  and partial I/O failure with exact already-written paths.
- `tests/gen.rs` verifies operation authority, deterministic provenance, and
  validation failures without publication.
- `tests/edit.rs` checks draft identity/shape failures and structured error codes.
- `tests/source_validation.rs` and `tests/edit_source_selection.rs` distinguish
  safely locating an old invalid value from validating its replacement.

Extend these current consumers when a new operation needs them. Do not introduce
an adapter registry, universal package snapshot, or execution state machine before
an actual producer and consumer require that boundary.
