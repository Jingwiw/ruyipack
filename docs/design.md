<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# Ownership and validation boundaries

RuyiPack calculates candidates before publishing them. It does not need a package
workspace database or a persistent package-wide mode to do so.

## Authority belongs to the operation

- WORK binds the package, current generation input, checkout and stage location.
- `gen` consumes the explicitly selected authoring TOML or source-bound stage.
  Authoring input renders a recipe; a source-bound stage patches its immutable SPEC
  baseline. Metadata chooses the path, not a filename/key heuristic.
- `edit` stages supported fields through one TOML document, whether edited by editor or CLI. It does not publish SPEC
  until `--apply`; `--check` and `--diff` are independent requested operations.
- Candidate SPEC, resolved TOML and diff are derived outputs, never implicit inputs. Candidate construction
  always reparses and compares fields; cached approvals are never trusted.
- `spec::candidate` is the shared producer for edit and source-bound generation.
  Mandatory mapping/parse/readback safety is separate from optional static checks.
- Source changes invalidate the stage; `--force` cannot bypass this check. In-place
  publication advances the baseline only after observed SPEC bytes match. Baselines
  are immutable and indexed by hash so a failed index update retains recovery bytes.
- Reports describe actual inputs, outputs and checks. Unchecked is not passed;
  static validity and local-edit admission are separate facts. No automatic commit.

## Current producer and consumer boundaries

| Producer | Value | Consumer |
| --- | --- | --- |
| `build::Engine` | Staged invocation and engine-specific result verification | `build` composes it with a backend |
| `build::Backend` | Execution, input/output transfer, cleanup and transport evidence | `build` then asks the engine to validate returned artifacts |
| `render` | Generated contents, static report, selected build contract | `generate` previews, reports, or publishes |
| `spec::document::Snapshot` | Original bytes, selected fields and replacement ranges | `spec::candidate` calculates and validates an edit |
| `spec::candidate` | Candidate contents, optional static report, review triggers | `edit` checks, previews, or publishes |
| `file_output` | Written/unchanged/skipped paths, typed partial failures | `edit` formats results and retains recovery information |
| `check` | Findings and explicit incomplete reasons | `check`, `gen`, and `edit` reports |
| `profile::buildsystems` | Embedded system names, requirements, stage guidance and identity | `new`, manifest validation, generation reports and RPK004 |
| `spec::sources` | Source/Patch identities, original expressions, declared digests and static values | Source hashing/verification and selected URL validation; `check --materials` inventories staged files |
| `source` | Resolved URLs and digests of actual downloads | `gen` completes missing digest facts; `edit` refreshes requested candidate digests; `source hash` calculates and `source verify` compares without writing |
| `report` | Input identity, numbered materials and error presentation | CLI reports and resolved snapshots |
| `stage` | Persistent input, immutable baselines, candidate/diff cache and output protection | `edit` and `gen` |

Read `generate.rs` and `edit.rs` for orchestration, then `render.rs` and
`spec/candidate.rs` for calculation. `spec/document.rs` owns the snapshot;
`document/capture.rs` maps selected fields and `document/render.rs` validates and
applies replacements. `file_output.rs` owns conflict handling and publication. `output_cli` owns argument translation and
terminal conflict selection; publication retains the checks around that selection.
CLI/editor interactions stay outside candidate calculation. `source_hash` only
presents the explicit hashing command; generation and editing consume `source`
directly. `spec::files` parses file rows for both manifest validation and result
comparison; input validation does not call the output verifier. These are private
modules, not a promised Rust library API.

Edit execution returns per-file checks and the publication result, including
partial failures. TOML reporting borrows these results; draft retention and failed-output
recovery use those same facts. Candidate verification borrows its rendered text;
only a long-lived edit snapshot takes ownership of the source.

## Facts, decisions, and evidence

`spec/` adapts parser syntax and source locations into the limited views the tool
uses. Syntax is not a macro-expanded RPM value. Unsupported or ambiguous selected
fields must not be guessed. Packaging policy remains in the rendering/checking
code and embedded profile contracts, not in an upstream parser's data model.

Reports identify exact inputs and the actual parser. `gen --check` additionally
identifies its manifest and selected profile/build contract; `edit --check`
distinguishes original and candidate hashes. Profile hashes cover embedded TOML,
not every validation rule or a target environment. Report paths are display text
(lossy for non-UTF-8 paths), not replayable file identities; hashes identify the
bytes actually checked. File operations retain native paths.

The `spec-static` stage only proves selected static checks. Native checks in
`scripts/check-native-sources` are separate evidence: Source-numbering parses do
not prove a build; the optional generated fixture proves only its bounded build.
OBS release/changelog services are not tested by its explicit fixture macro values.
Build evidence needs the actual source, environment, command,
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
| Source SHA-256 | openRuyi requires digests for HTTP(S) sources. RPK005 warns about missing or malformed digests in static checks; authoring or directly editing a digest rejects malformed values. A static pass is not policy certification. |
| BuildSystem / BuildOption and stage hooks | RPM declarative build syntax; actual actions come from target macros. `render::spec` emits declarations, not copies of default scripts. |
| Autotools tool requirements | `check::build` enforces the profile declaration contract, not a dependency solver. CMake/Meson empty lists do not assert dependency-free builds. |

The [build-system TOMLs](../profiles/openruyi/buildsystems) record macro-file
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

CLI tests share `tests/integration/main.rs`, with domain modules that can be run
separately (for example, `cargo test --test integration stage::`). Editing
separates draft, editor, and report scenarios; generation separates build stages
and package structure. Their command-local fixtures remain in the parent modules.
Closed-stream
regressions remain isolated in `tests/stdio.rs`. Argument-conflict matrices use
`Cli::try_parse_from`; CLI cases retain exit-status and file-safety checks.

- `spec::document` property tests and `tests/integration/edit/selection.rs` check
  exact preservation outside selected fields, including unsupported syntax.
- `file_output/tests.rs` tests no-op byte/inode/mtime preservation, stale-source rejection,
  and partial I/O failure with exact already-written paths.
- `tests/integration/generation.rs` checks input authority, provenance, and
  validation failures without publication.
- `tests/integration/edit/drafts.rs` checks identity/shape failures; `edit/reports.rs` checks error codes and publication receipts.
- `tests/integration/source_validation.rs` and `tests/integration/edit/sources.rs` distinguish
  safely locating an old invalid value from validating its replacement.
