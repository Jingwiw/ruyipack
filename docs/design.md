<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# Ownership and validation boundaries

RuyiPack constructs and validates candidates before publication.
Each operation selects its input explicitly.

## Authority belongs to the operation

- WORK binds one editable TOML at its root. `edit` and `gen` consume that same file.
- Authoring TOML renders a recipe. Imported SPEC fields use a saved original and
  selected byte ranges, preserving scripts and other unmapped content.
- `edit` changes TOML through the editor or CLI; only `--apply` publishes a SPEC.
  `--check` and `--diff` are independent requested operations.
- Candidate SPEC, resolved TOML and diff are derived outputs, never implicit inputs. Candidate construction
  always reparses and compares fields; cached approvals are never trusted.
- `spec::candidate` is the shared producer for edit and source-bound generation.
  Mapping, parsing and readback checks are mandatory. Static checks are a separate choice.
- Source changes invalidate its saved baseline; `--force` cannot bypass this check. In-place
  publication advances the baseline only after observed SPEC bytes match. Baselines
  are immutable and indexed by hash so a failed index update retains recovery bytes.
- Reports describe observed inputs, outputs and checks. Unchecked does not mean passed.
  Static validity differs from local-edit admission. Operations do not commit automatically.

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
| `source` | Resolved URLs and digests of actual downloads | `gen` completes missing digest facts; `edit` refreshes requested candidate digests; `source hash` calculates and `source verify` compares without changing declarations |
| `report` | Input identity, numbered materials and error presentation | CLI reports and resolved snapshots |
| `stage` | Persistent input, immutable baselines, candidate/diff cache and output protection | `edit` and `gen` |

Read `generate.rs` and `edit.rs` for orchestration, then `render.rs` and `spec/candidate.rs` for calculation. `spec/document.rs` owns the snapshot; `document/capture.rs` maps selected fields and `document/render.rs` validates and applies replacements. `file_output.rs` owns conflict handling and publication. `output_cli` owns argument translation and terminal conflict selection; publication retains the checks around that selection.
CLI/editor interactions stay outside candidate calculation. `source_hash` only presents the explicit hashing command; generation and editing consume `source` directly. `spec::files` parses file rows for both manifest validation and result comparison; input validation does not call the output verifier.
These are private modules, not a promised Rust library API.

Edit execution returns per-file checks and the publication result, including partial failures.
TOML reporting borrows these results; draft retention and failed-output recovery use those same facts.
Candidate verification borrows its rendered text; only a long-lived edit snapshot takes ownership of the source.

## Facts, decisions, and evidence

`spec/` adapts parser syntax and source locations into the limited views the tool uses.
Syntax is not a macro-expanded RPM value.
Unsupported or ambiguous selected fields must not be guessed.
Packaging policy remains in the rendering/checking code and embedded profile contracts, not in an upstream parser's data model.

Reports identify exact inputs and the actual parser. `gen --check` additionally identifies its manifest and selected profile/build contract; `edit --check` distinguishes original and candidate hashes.
Profile hashes cover embedded TOML, not every validation rule or a target environment.
Report paths are display text (lossy for non-UTF-8 paths), not replayable file identities; hashes identify the bytes actually checked.
File operations retain native paths.

The `spec-static` stage only proves selected static checks.
Native checks in `scripts/check-native-sources` are separate evidence: Source-numbering parses do not prove a build; the optional generated fixture proves only its bounded build.
OBS release/changelog services are not tested by its explicit fixture macro values.
Build evidence needs the actual source, environment, command, and resulting artifacts; absent evidence is not success.
No universal success flag spans these stages.
Warnings do not imply incomplete checks.
Each rule reports unresolved checks explicitly.
Reasons remain visible even when another violation makes the overall result fail.

## openRuyi policy sources

The embedded profile is a supported authoring subset, not a full implementation of all distribution policy.
Its defaults come from the pinned [packaging specification](https://github.com/openRuyi-Project/homepage/blob/9862c6a93a0068a30d5b29cad74d14a281ca2e66/docs/packaging-guidelines/rpmspecification.md).

| Convention | Owner and implementation |
| --- | --- |
| SPEC header license vs package License | `profile.toml` supplies the former; manifest `package.license` supplies the latter. Do not apply MulanPSL to upstream software by default. |
| `%autorelease` / `%autochangelog` | Profile output remains literal; release/history handling belongs to target macros, not the renderer. |
| RemoteAsset | openRuyi fetch metadata in a comment attached to a Source; `profile` owns spelling, `spec::document` owns safe adjacency/ranges, `source` validates selected values. |
| Source SHA-256 | openRuyi requires digests for HTTP(S) sources. RPK005 warns about missing or malformed digests in static checks; authoring or directly editing a digest rejects malformed values. A static pass is not policy certification. |
| BuildSystem / BuildOption and stage hooks | RPM declarative build syntax; actual actions come from target macros. `render::spec` emits declarations, not copies of default scripts. |
| Autotools tool requirements | `check::build` suggests explicit declarations from the profile; never a build gate or dependency solver. CMake/Meson empty lists do not assert dependency-free builds. |

The [build-system TOMLs](../profiles/openruyi/buildsystems) record macro-file paths and revisions beside their copied stage guidance.
Consult those sources before changing defaults; verify actual macros in the target environment. [Native RPM semantics](https://rpm.org/docs/6.0.x/manual/spec.html) and openRuyi policy are separate.
Implicit Source numbering comes from RPM. RemoteAsset association is a distribution convention.

## Publication and recovery

`file_output` keeps exact paths in typed outcomes and partial failures.
The [output contract](reference.md#output-and-file-safety) defines per-file atomicity, permission handling, stale-source checks, and their limits.

## Regression contracts

CLI tests share `tests/integration/main.rs`, with domain modules that can be run separately (for example, `cargo test --test integration edit::`).
Editing separates draft, editor, and report scenarios; generation separates build stages and package structure.
Fixtures remain in their parent modules.
Closed-stream regressions stay in `tests/stdio.rs`.
Argument-conflict matrices use `Cli::try_parse_from`; CLI cases retain exit-status and file-safety checks.

- `spec::document` property tests and `tests/integration/edit/selection.rs` check
  exact preservation outside selected fields, including unsupported syntax.
- `file_output/tests.rs` tests no-op byte/inode/mtime preservation, stale-source rejection,
  and partial I/O failure with exact already-written paths.
- `tests/integration/generation.rs` checks input authority, provenance, and
  validation failures without publication.
- `tests/integration/edit/drafts.rs` checks identity/shape failures; `edit/reports.rs` checks error codes and publication receipts.
- `tests/integration/source_validation.rs` and `tests/integration/edit/sources.rs` distinguish
  safely locating an old invalid value from validating its replacement.

## Development

Run these commands from the source repository, not from `docs/`.
Review existing hooks before enabling the local pre-commit hook.

```sh
git config --local core.hooksPath .githooks
./scripts/check-rust
./scripts/check
./scripts/smoke-test "${CARGO_HOME:-$HOME/.cargo}/bin/ruyipack"
```

`scripts/check-rust` is the shared pre-commit and CI Rust gate.
It checks formatting, denies Clippy warnings and runs behavioral tests for the workspace, all targets and all features.
The hook checks a fixed snapshot of staged files.
Partial staging and unrelated working changes are allowed.
It does not stash or rewrite files or the index.
If staged contents change during checks, the hook refuses the commit and requests a retry.
Staged builds use `target/pre-commit` unless `CARGO_TARGET_DIR` is set.

`Cargo.toml` owns the lint policy: default Clippy rules plus selected checks for unnecessary ownership/cloning, avoidable string allocations, and lossy integer conversions. `clippy.toml` forbids process-output methods in `file_output`.
Printing and debug macros are also forbidden there. Callers supply the writer; CLI modules may acquire stdout/stderr.
Broad `pedantic` is an advisory review, not a zero-warning gate:

```sh
cargo clippy --workspace --all-targets --all-features --locked -- -W clippy::pedantic
```

Fix the cause before suppressing a lint; any necessary exception should be local and explain the invariant.
Keep the pinned toolchain for the commit gate. `scripts/check-tests` runs the full suite, then repeats only unit tests in a color-capable pseudo-terminal.
It requires Python 3 on Linux/macOS (standard library only) and is shared by pre-commit, pinned CI and stable CI.
Output stays terse; terminal failures propagate instead of being hidden by CI pipes.

For refactors, inspect rust-analyzer references, implementations and callers up to CLI entry points; separate production consumers from tests.
Reference counts are review evidence, not pass/fail thresholds.

`scripts/check` additionally runs schema/gen comparisons, REUSE, and cargo-deny.
Install Python 3.11+ with `reuse==6.2.0` and `jsonschema==4.26.0`, plus `cargo-deny` 0.20.2 separately.
The smoke test exercises the installed binary in temporary files.

CI also runs `cargo machete`, pinned in the Check workflow.
Only the tool is cached; every run scans dependencies without `--fix`.
Pre-commit does not require it.
Review findings before removing dependencies: use `package.metadata.cargo-machete.renamed` for import-name mismatches.
Any necessary `package.metadata.cargo-machete.ignored` entry must have an adjacent comment identifying its real consumer and why the scanner misses it.

In a trusted target environment with Python 3, `rpm`, `rpmspec`, and `rpm-config-openruyi`, `./scripts/check-native-sources` checks native Source numbering and records RPM/macro identities.
Pass prepared SPEC paths and `--require-package NAME` for additional parses, or `--ruyipack /path/to/ruyipack` to generate and build the small local semantic fixture.
CI runs this gate in the pinned environment in `tests/native/Dockerfile`, offline and without root.
It checks Patch/stage order, subpackage ownership, file lists and flags—not ecosystem build coverage or OBS release/changelog services.
Parser diagnostics, including those with exit status 0, prevent acceptance.
Removed pinned packages require an explicit environment update and revalidation, not a skipped gate.

Keep changes focused and include a regression test for behavior changes.
For bug reports, include `ruyipack --version`, OS/architecture, the exact command, minimal input, and complete output.
Remove credentials and private data before sharing.
