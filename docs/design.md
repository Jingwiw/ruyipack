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
| `check` | Findings and explicit incomplete reasons | `check`, `gen`, and `edit` reports |
| `profile::buildsystems` | Embedded system names, requirements, stage guidance and identity | `init`, manifest validation, generation reports and RPK004 |

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
flag spans these stages. Rule warnings do not imply incompleteness: each rule
reports unresolved checks explicitly. All incomplete reasons remain visible even
when a confirmed violation makes the overall result fail.

## openRuyi policy sources

The embedded profile is a supported authoring subset, not a full implementation of
all distribution policy. Its defaults come from the pinned
[packaging specification](https://github.com/openRuyi-Project/homepage/blob/9862c6a93a0068a30d5b29cad74d14a281ca2e66/docs/packaging-guidelines/rpmspecification.md).

| Convention | Owner and implementation |
| --- | --- |
| SPEC header license vs package License | `profile.toml` supplies the former; manifest `package.license` supplies the latter. Do not apply MulanPSL to upstream software by default. |
| `%autorelease` / `%autochangelog` | Profile output remains literal; release/history handling belongs to target macros, not the renderer. |
| RemoteAsset | openRuyi fetch metadata in a comment attached to a Source; `profile` owns spelling, `spec::document` owns safe adjacency/ranges, `source` validates selected values. |
| BuildSystem / BuildOption and stage hooks | RPM declarative build syntax; actual actions come from target macros. `render::spec` emits declarations, not copies of default scripts. |
| Autotools tool requirements | `check::build` enforces the profile declaration contract, not a dependency solver. CMake/Meson empty lists do not assert dependency-free builds. |

The [build-system TOMLs](../profiles/openruyi-v1/buildsystems) record macro-file
paths and revisions beside their copied stage guidance. Consult those sources
before changing defaults; verify actual macros in the target environment.
[Native RPM semantics](https://rpm.org/docs/6.0.x/manual/spec.html) and openRuyi
policy are separate: implicit Source numbering is RPM behavior, whereas
RemoteAsset association is a distribution convention.

## Publication and recovery

`file_output` keeps exact paths in typed outcomes and partial failures. The
[output contract](reference.md#output-and-file-safety) defines per-file atomicity,
permission handling, stale-source checks, and their limits.

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
