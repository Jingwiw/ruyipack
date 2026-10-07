<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# Command and manifest reference

[Quick start](../README.md#try-it) · [Design and policy sources](design.md)

Use `ruyipack COMMAND --help` for option syntax.
This reference explains behavior, constraints and report fields.

## Workspace initialization

`init` reads global Git `user.name` and `user.email` once.
It saves `author = "Name <email>"` in `.ruyiconfig/config.toml`.
Missing or invalid identity leaves an empty value and produces a warning with the configuration path. `new` uses this saved default.
Repeated initialization and later Git changes do not replace it.
Existing and imported TOML retain their own `spec.contributors`.

`ruyipack init [PATH]` writes workspace and embedded build configuration to an empty directory.
PATH defaults to the current directory.
Without `--clone`, Git, Docker and network access are optional.
Initialization does not start Docker or create a package.
An existing marker produces a warning, even after interrupted or damaged initialization.
The command does not repair or overwrite existing files.

The configuration defines three paths:

- `recipes`: defaults to `openruyi`; external paths are allowed.
- `work`: defaults to `work`; it must stay inside the workspace.
- `specs`: defaults to `SPECS`, relative to the recipe repository.

Re-running bare `init` diagnoses invalid base configuration without requiring the recipe repository or build tools.

With `--clone URL`, initialization finishes before Git clone starts in the configured `recipes` path.
Clone failure exits 1 and retains the configuration and files Git left behind.
Diagnostics distinguish successful initialization from failed cloning.
Bare `init` never retries clone.
To retry, use `--clone` with valid configuration and an absent destination.
Any existing destination is refused, including empty or interrupted directories.
Inspect retained files before retrying.
Invalid workspace configuration blocks clone; it is not repaired.
Git clone has a 300-second execution budget; other Git calls have 30 seconds.
User Git configuration, filters, and SSH/credential helpers remain trusted; this is not a sandbox.

## New development area and authoring input

`new WORK` creates a local package development area; no recipe repository is required. `--pkgname PKG` sets the package binding (otherwise WORK).

```text
work/WORK/
  .config.toml      # package and recipe/authoring kind, fixed at creation
  PKG.toml          # authoring input for a scaffold or TOML import
  recipe/SPECS/PKG/ # SPEC, Patch and other local recipe files
  sources/          # downloaded material cache
  build/            # retained build results
```

Choose one source:

```sh
ruyipack new example --build-system cmake
ruyipack new review --from-toml existing-authoring.toml
ruyipack new review --from-dir /path/to/SPECS/ed
ruyipack new spec-only --from-spec /path/to/SPECS/ed/ed.spec
```

`--from-toml` copies an existing RuyiPack authoring TOML verbatim, including comments.
Its `package.name`, when present, supplies PKG.
Partial TOML is accepted; `gen` validates recipe facts.
This is not an importer for Cargo.toml or upstream metadata.

WORK names the development area. The three `--from-*` options are mutually exclusive input sources.
Without a source, PKG defaults to WORK. TOML supplies `package.name`; SPEC sources supply the filename stem, not the parent directory or evaluated Name.
`--pkgname` overrides only the package directory binding. It does not choose a SPEC or rewrite imported facts; a conflicting TOML `package.name` is rejected.
Existing WORK bindings cannot be changed by repeating `--pkgname` on another command.
`gen` consumes the saved WORK binding and has no `--pkgname` option.

`--from-dir DIR` copies all ordinary files from a package directory.
`--from-spec PATH` copies only the SPEC, not adjacent materials.
Directory import requires exactly one top-level SPEC, even if one matches the directory name.
No SPEC or multiple SPEC files is an error; use `--from-spec` to select a file explicitly. Import preserves the SPEC filename and Name; mismatches produce warnings.
It writes supported editable fields to `work/WORK/<SPEC-stem>.toml`.
The retained SPEC and baseline preserve scripts, macros and unmapped content. `edit WORK` and `gen WORK` use this same source-bound input.
The projection is not a standalone manifest.

Import rejects symlinks, special files, filename collisions and an existing WORK.
It does not copy files outside the package directory.
Run `check WORK --materials` to find unresolved or missing Source/Patch inputs.
Import does not establish build success.

Imports default to the declared package name; WORK only names the development area.
Directory imports use the directory basename; SPEC-only imports use the filename stem. `--pkgname` overrides this binding without rewriting `Name`. A different or unresolved SPEC Name produces a commit warning, not a directory-binding error. Required submit checks and material checks still apply.
Original SPEC bytes are retained until publication.
Literal script, URL and Patch strings are never mass-renamed.
TOML imports stay verbatim and reject a conflicting `--pkgname`.
Original inputs are not modified or synchronized afterward. `--build-system` and `--comments` are scaffold-only options. `new` never downloads source materials.

`--stdout` and `--diff` preview without creating WORK.
Ordinary scaffold/TOML output conflicts use `--force` or `--skip-existing`; neither changes WORK's binding or kind. `gen --apply` publishes to the selected recipe directory. `edit`, `check`, `build` and `open` consume that same directory, independent of Git.

## Commit package changes

Use a personal fork for commit/PR delivery. Do not configure the openRuyi project
main repository as the delivery origin or push destination. The project main
repository may be the PR base target, not the source branch destination.
Local checks and builds do not require a personal repository address.

`commit WORK` applies package changes to the configured recipe repository's current branch.
Use `--repo PATH` to select another repository, `--dry-run` to inspect the diff,
`--spec-only` to commit only the bound SPEC, or `-m MESSAGE` to set the message.
The target repository must have a clean worktree and index. Detached HEAD is rejected.
Conflicting target changes stop before publication. Files absent from a SPEC-only import are not deleted.

Commit checks static submit rules and local Source/Patch files in the planned delivery.
`--spec-only` cannot omit a required local material absent from the target repository.
Previews include file creation, deletion and executable-mode changes; `admission.allowed`
reports submit readiness separately from preview success.
The build report distinguishes `not-run`, `unavailable`, `stale`, `failed` and `passed`,
with the recorded stage and environment. `passed` at `prep` is not a complete build.
Missing or failed build evidence does not prevent a local commit. Commit does not download or build. It does not generate a SPEC,
switch branches, push, or open a PR. Git supplies commit identity, hooks and signing. `--timeout SECONDS` bounds the commit, including hooks and signing (default: 300 seconds).
If Git rejects a commit, package changes remain in the target repository. Fix the cause and
repeat the same command. Unrelated edits block retries; RuyiPack never resets them.
A created commit is reported even if saving the final WORK state fails.

`delete WORK` lists recipe, authoring and build files for confirmation; `--force` skips confirmation.
It removes the receipt-bound OBS package and owned build resources before WORK files.
OBS projects and shared images remain. Another WORK binding keeps a shared OBS package.
Changed remote files or missing ownership evidence block deletion; `--force` does not bypass these checks.
Use `clean WORK` for local builds or `clean WORK --remote` for OBS; both keep WORK files. `--dry-run` reads local records only.
It does not remove Git commits or branches. `clean WORK` keeps authoring and recipe files.

## Common SPEC selection

Human diagnostics identify the input once, then use `spec[LINE:COLUMN]` (or `spec` when no location is known).
TOML retains full paths and source positions.

A positional argument always selects WORK.
The first implicit package lookup requires committed `SPECS/WORK/WORK.spec`; `--pkgname PKG` explicitly binds another package.
Without a match or explicit binding, the command fails before allocation.
Existing WORK uses its saved recipe directory and package binding.
A conflicting `--pkgname` fails.
Local areas do not consult Git.

Before package files exist locally, reads use committed main.
`inspect`, `check`, `schema edit` and read-only Source queries create only the WORK binding.
Reports identify the read commit as `input.revision`.
`edit`, `open`, `source fetch` and `build` copy the committed package files into
`work/WORK/recipe/SPECS/PKG/` when needed. They do not create branches.
Uncommitted files in the source repository are not imported by this path.
Use `new --from-dir DIR` to copy those files explicitly.
Existing local recipe files remain authoritative even when the source repository is unavailable.
Use `edit --expect-sha256 HASH` to pin the bytes you reviewed.

Use `--spec PATH` for external files.
This is explicit even for `.spec` filenames; file existence never changes argument meaning. `check --manifest PATH` and `source verify --manifest PATH` select authoring input instead.
These modes are mutually exclusive, not fallback searches.

## Manifest editor schema

Export the authoring schema with `ruyipack schema manifest > ruyipack.schema.json`.
The command reads no package and downloads nothing; the shell writes its stdout to the file.
Add this directive at the start of the manifest, followed by a blank line:

```toml
#:schema ./ruyipack.schema.json

[package]
# ...
```

[Tombi and compatible TOML editors](https://tombi-toml.github.io/tombi/docs/json-schema/) use it for completion, field descriptions and structural diagnostics.
Paths are relative to the manifest.
Regenerate the local schema after updating RuyiPack; no mutable remote URL or network access is required. `new` does not create a sidecar or reference a file that may not exist.

`schema edit WORK --field FIELD` describes selected fields in an existing SPEC instead.
The authoring schema checks fields, types, build/stage names, material shapes and supplied SHA-256 syntax.
Missing SHA-256 is allowed.
Unfilled scaffolds can pass these structural checks but fail generation.

Run `ruyipack gen WORK --offline --check` for cross-field constraints.
These include RPM expressions, numeric aliases/overflow, Source0, VCS choices and file-list content.
Schema success is not build validation.

## Generate

`gen WORK` reads the WORK's single TOML: `work/WORK/PKG.toml`,
or the selected SPEC filename stem when importing a differently named SPEC.
`edit` updates the same file. Imported inputs retain their original SPEC and fixed
mapping in `.state/`; changed source bytes are rejected rather than guessed.

Generation writes resolved TOML, candidate SPEC and diff under `work/WORK/.cache/`.
These are output artifacts, never inputs. Generation recomputes them from the current TOML.
The read-only snapshot uses standard TOML, format version 2.
`manifest` contains validated renderer values. `profile` contains target defaults. `downloads` contains observed download results.
It preserves RPM expressions rather than claiming native macro expansion.
Source-bound snapshots instead have an `edit` section with the immutable source identity, selected fields and resolved values; unsupported scripts stay in SPEC.
Neither snapshot is an authoring manifest or an automatic input to `gen`.

`--output=.` writes `PKG.spec` beside the snapshot; `--apply` publishes the recipe SPEC; `--output=PATH` names a complete destination.
Publications use static admission (the same local-edit gate for source-bound input). `--check` writes no snapshot, candidate or SPEC. `--stdout` prints the candidate. `--diff` shows changes; adding `--apply` publishes that same checked candidate.
Safely constructed candidates remain diffable after static failure (exit 1, no publication).
Preview and check can download materials.
Use `--offline` to prevent downloads.
See [ed.toml](../examples/ed/ed.toml) for authoring input.

### Repository and Sources

Choose an entry in `[package.vcs]` only when the fact is known:

| Entry | Generated result |
| --- | --- |
| `git = "https://example.org/project.git"` | `VCS: git:...`; do not include the prefix in the input |
| `same-as-url = true` | Omit VCS; `package.url` already names the source repository |
| `no-public-repository = true` | Emit the profile's no-repository comment |

Omit the table or leave it empty while the repository status is unknown.
Generation warns (also in TOML `authoring_warnings`), emits no VCS assertion, and never turns an unknown or failed lookup into `no-public-repository`.
Conflicting choices fail.
Addresses and declarations are not verified remotely.
A generated candidate is not proof of VCS policy compliance; confirm the declaration before publishing.
The ed example illustrates an explicit declaration, not verified repository evidence.

Each `[sources.N]` chooses exactly one material form: `url` with optional `sha256` for a remote resource, or `path` for a local input. `[patches.N]` accepts a local `path` only.
Local paths receive no RemoteAsset marker or hash; gen does not read, copy, or confirm these files.
URL/path combinations and hashes on local material fail.
Source and Patch numbers have independent namespaces.
Sources are sorted numerically; Patches retain TOML declaration order, because native `%autopatch` uses that order.
Patch declarations are placed after BuildSystem; application, strip level, and patch order remain the responsibility of the prep stage/declarative RPM machinery.
Follow openRuyi's four-digit patch filename categories and document patch purpose in its header; generation does not inspect patch content or test applicability.

For example:

```toml
[sources.1]
path = "example.service"
[patches.2000]
path = "2000-fix-build.patch"
```

`sources.0` is the primary archive used by the Autotools default unpacking step.
A missing digest produces a bare `#!RemoteAsset` and warning, not an error.
This output does not meet openRuyi's SHA-256 requirement for HTTP(S) sources.
An empty digest fails.
Digests must be 64 hexadecimal digits; case is preserved.

After filling the scaffold, `gen WORK` automatically attempts to download remote Sources whose `sha256` is absent and fill the generated SPEC.
Use `--offline` to skip downloads entirely.
Existing digests are retained, not downloaded or verified; remove a digest from the TOML when you intend to recalculate it.
Completion uses built-in Rust HTTP/TLS and the package fields below, without executing macros.
Download failures warn per Source and leave its digest missing.
Connection setup is limited to 10 seconds; the complete transfer, including redirects and reading the body, to 300 seconds.

The resolved snapshot contains the validated values used for rendering; author input remains unchanged. `--check --format toml` reports downloads in `source_hashes`, failures in `source_hash_failures` and warnings in `authoring_warnings`.
With `--offline`, `source_hashes` is empty.
Explicit `--hash` recalculates existing declarations; ordinary completion preserves them.
A digest is an observation of downloaded bytes, not source authentication.
A failed refresh after a version or URL change produces a warning.
An old declaration is not reported as a newly calculated digest.

Source expressions may use `%{name}`, `%{version}`, and `%{url}` only when the referenced package values are unambiguous static literals.
Expressions and filename fragments such as `#/archive.tar.gz` remain in the SPEC.
Literal percent escapes use `%%` (for example `a%%20b.tar.gz`); other macro tokens are unsupported.
New manifests require HTTPS; editing permits existing HTTP Sources.
Authored URLs reject userinfo credentials.
This is not a secret scan or redaction service. Query strings, views and diffs can still contain secrets.

### Build stages

Stages are `prep`, `conf`, `build`, `install`, and `check`, emitted in that order.
With `build.system`, RPM supplies default actions; the following fields customize one stage:

```toml
[build.stages.conf]
prepend = 'autoreconf -fiv'
options = ["--enable-largefile", "--enable-nls"]

[build.stages.install]
append = '''
rm -f %{buildroot}%{_infodir}/dir
'''
```

| Field | Meaning |
| --- | --- |
| `options` | Ordered single-line strings emitted as `BuildOption(stage):  ...`; no automatic shell-argument quoting |
| `prepend` / `append` | `%stage -p` / `%stage -a` scripts around the main action; empty strings emit nothing |
| `replace` | `%stage` main action; omitted preserves the default, `""` emits an empty section to skip it |

Nonempty `options` and `replace` cannot coexist in one stage.
Put arguments in the replacement instead.
Hooks still surround a replaced action.
Scripts require LF; indentation, comments, and macros are preserved.
Keep reasons for skipped tests in script comments.
Validation checks boundaries and text, not shell correctness.

Without `build.system`, supply explicit scripts, for example:

```toml
[build.stages.prep]
replace = '%autosetup -p1'
[build.stages.build]
replace = '%make_build'
[build.stages.install]
replace = '%make_install'
```

There are no default stages, including unpacking; omitting `[build]` emits none. `options` requires a system.
Declare tools in `[build-requires]`; no build-system requirements are added or enforced in this explicit mode.

### Manual subpackages

```toml
[subpackages.devel]
summary = "Development files for %{name}"
description = "Headers for %{name}."
requires = ["%{name} = %{version}-%{release}"]
provides = ["%{name}-development = %{version}-%{release}"]
[subpackages.devel.files]
entries = ["%{_includedir}/%{name}.h"]
```

A subpackage key is a suffix: `devel` produces `<main-name>-devel`.
With `full-name = true`, the key is the complete name.
The renderer then uses `-n` in `%package`, `%description` and `%files`.
Names must be literal RPM names with no resulting main/subpackage collisions.
Each subpackage needs nonempty summary and description.
Requires and Provides use the main package's expression rules; files accept `license`, `doc`, `entries`, and `lists`.
Empty/omitted subpackage files emit an empty `%files` for a metapackage; the main package needs at least one entry or external list.

Per-subpackage license/URL/architecture, conditional declarations, and macro-generated families are unsupported.
File ownership still needs build-time verification.

### Native file entries and generated lists

Main and subpackage `files` use the same fields. `license` and `doc` remain shortcuts; ordered `entries` also accept native file rows such as `%dir`, `%config(noreplace)`, `%ghost %attr(0644,root,root)`, `%verify`, `%exclude`, and `%defattr`.
Do not put conditions, sections, or arbitrary macro statements there.
The renderer preserves row order and the verifier compares directives and paths.
The `license`/`doc` shortcuts are emitted before `entries`.
When order matters (for example around `%defattr`), put all affected rows in `entries` instead.

```toml
[package.files]
license = ["COPYING"]
lists = ["%{name}.lang", "generated.files"]
entries = ["%dir %{_datadir}/example", "%config(noreplace) %{_sysconfdir}/example.conf"]
```

`lists` emits repeated `%files -f` arguments, without guessing the package name or adding an implicit list.
It can be the only content of a files section.
These lists are produced/read during the RPM build, never opened by gen.
File existence, macro expansion, final ownership, and list contents still need native build validation. `%files -l` and conditional file sections are not supported.

## Edit

WORK selection follows [the common input rules](#common-spec-selection).
Use `edit --spec PATH` for independent files; repeat `--spec` for a batch. `--from` retains its saved absolute SPEC paths.
Named operations hold their WORK lock through validation and publication; direct-file edits do not use that lock.

`open WORK --authoring` opens the existing authoring TOML, without generating a SPEC or applying saved edit values.
A missing authoring file is an error.

`open WORK` opens the package recipe directory for SPEC/Patch editing.
It retains the WORK lock while the editor runs.
Edits are direct: no TOML changes, automatic validation, Git staging or commit; field/hash/apply options cannot be combined with this mode.

Default `edit WORK` opens the WORK TOML.
Both modes select the editor through `--editor`, then `editor` in `.ruyiconfig/config.toml`, then `git var GIT_EDITOR` (in the source repository; a batch uses its first SPEC).
Git owns the remaining `GIT_EDITOR` / `core.editor` / `VISUAL` / `EDITOR` / default precedence.
Editor values are trusted shell commands, just like Git's `core.editor`; file paths are separate arguments.
No editor-specific flags are injected.
GUI editors must wait, e.g. `editor = "code --wait"`.
Output goes to stderr, and a failed editor never causes automatic publication.
No menu, static check or SPEC write happens implicitly.
Unfinished TOML remains saved even when it cannot yet generate a candidate.

`--menu` selects fields and edits their current values inline. `--set FIELD=VALUE` is non-interactive.
All field-editing paths save the same TOML document; `open WORK` edits files directly. `--field FIELD` edits the current value directly in the terminal; use `--editor COMMAND` for selected TOML editing, or `--prepare DIR` to save it without opening an editor. `--all` requests strict full mapping. `inspect WORK --editable --field FIELD` and `schema edit WORK --field FIELD` are separate read-only projection/schema queries.

`--diff` constructs a candidate with the shared generation logic and caches the SPEC.
It saves a `.diff` under WORK `.cache/`; explicit-file drafts keep the diff beside their TOML.
Human output shows the diff; TOML includes its path. `--check` additionally checks the edited candidate. `--apply` always checks local admission and publishes only when admissible.
These options can be combined. `--hash` refreshes every remote HTTP(S) Source, including signatures; `--hash-source N` selects individual Sources.
Both use the edited candidate and save the resulting facts back to the TOML.
Local materials/Patches are excluded.

The subset covers existing main-package metadata, Sources with adjacent RemoteAsset markers, BuildSystem/BuildRequires, descriptions, simple file lists, header metadata, comments, and changelog text.
Full views require every construct to be mapped; VCS tags and build scripts need a selected-field view.
Unselected bytes stay intact.
Parser errors or ambiguous selected fields stop editing.
Deleting keys or adding unmapped groups fails; supported existing lists can change.
Appending BuildRequires preserves comment-separated groups; other nonempty size changes require a contiguous group.

Unnumbered `Source:` uses its effective RPM number: after `Source3:`, it is 4, not 0.
Conditions, includes, or unsupported expressions can make implicit numbers uncertain; affected Source edits fail rather than guess.
Unrelated fields may still be edited.
Recognizable invalid URL/digest values can be viewed and repaired if their ranges are unambiguous; selected replacements must validate.
Unselected digests are preserved, not certified.
A bare RemoteAsset exposes an empty `sha256` value; `--set sources.0.sha256=HASH` adds a validated digest to that same marker.
An empty value preserves a bare marker but cannot erase an existing digest.

Version or Source URL changes produce `review_triggers` and `review_required` (source/digests, patches, native build).
They do not turn a static pass into a failure.
Empty lists mean no triggering change, not that external checks ran.
Use `--expect-sha256 HASH` for a single-file edit based on an earlier read or `source hash` result.
A mismatch refuses the operation; publication still checks for subsequent changes.
This binds the SPEC bytes, not external macros or URLs.

### Persistent drafts

```sh
ruyipack edit --spec ed.spec --spec other.spec --field package.version --prepare drafts
# Edit the TOML files in drafts, then:
ruyipack edit --from drafts --check --format toml
ruyipack edit --from drafts --diff
ruyipack edit --from drafts --apply
```

`--from` resumes without an editor; choose `--check`, `--diff` or `--apply`, or add `--editor COMMAND` to reopen TOML.
Drafts use source basenames and TOML 1.1.
Keep `.state` (original bytes, source identities, and schemas) with the editable files.
Duplicate basenames need separate directories.
A successful in-place apply advances the saved baseline.
External source changes require a fresh source-bound draft; `--force` never overrides that boundary.
Editor work is retained after validation failure or when changes remain unapplied.

## Output and file safety

Editing accepts LF and CRLF. TOML values use LF; replacement blocks use the local SPEC line ending.
Untouched bytes remain unchanged. Bare CR and NUL are rejected.

- `gen` defaults to resolved TOML and a cached SPEC; `new` writes `PKG.toml` inside
  the selected development area.
  `edit` defaults to persistent staging, and requires `--apply` to replace SPEC. Explicit output paths are relative to
  the current directory; parent directories must exist.
- `--stdout` prints one candidate without consulting the target. `--diff` prints
  a unified diff. `--apply` publishes the recipe; gen can also publish to explicit `--output` paths.
  Edit saves candidate/diff files. Edits can preview a batch. A diff is not a successful check:
  failed admission returns 1 and prevents publication. Successful previews return
  0 regardless of differences. Diff
  headers require UTF-8 paths without tabs/newlines.
- Different existing outputs require an explicit action or terminal confirmation.
  `--force` replaces; `--skip-existing` keeps existing files and creates missing
  ones. These apply to new/gen; edit's `--force` requires `--output`. Same-content
  writes leave files untouched. See `--help` for incompatible option combinations.
- TOML publication reports never prompt: conflicts require an explicit action.
  Without a usable terminal, unresolved human-mode conflicts also fail. The terminal menu offers
  keep (initial selection, still requires confirmation), diff, copy, or overwrite.
  Copies use `.new`, `.new.1`, etc. without replacing existing files. Cancellation
  fails without writing. Candidate/diff/TOML go to stdout; notices go to stderr.
- Edits reject detected source changes even with `--force`. All batch candidates
  validate before writing, but publication is atomic **per file**, not per batch.
  Later I/O failure reports already-written paths: inspect them before retrying.
  Successful batches report `Wrote`, `Unchanged`, or `Kept`; stderr failure does
  not undo published files. Notices are not a machine-readable apply journal.
- Unix replacements preserve ordinary permission bits (`0777`), clear special
  bits, and do not copy ownership/ACLs/xattrs. New files respect umask. Concurrent
  checks are best effort, not a lock against arbitrary writers; atomic replacement
  does not promise power-loss durability.
- Use only trusted local drafts: hashes prove consistency, not authorship. The
  configured editor is a trusted executable, not a sandbox.

## Local build materials

```sh
ruyipack check --spec package.spec --materials --source-dir ./SOURCES
ruyipack check --manifest package.toml --materials --source-dir ./SOURCES --format toml
```

`--materials` adds offline Source/Patch checks to the normal static checks; it never replaces them.
Manifest input is rendered and verified in memory, without hash downloads or output files.
Diagnostic positions then refer to the generated SPEC; TOML identifies both the original input and generated SPEC digest.

`--source-dir` requires `--materials`.
It selects the prepared RPM `_sourcedir`; by default this is the canonical recipe's directory, **not** the working directory.
Both local declarations and downloaded URL resources must already be staged there.
The filename follows RPM rules: the suffix after the last `/`, then after the last `=` in that suffix, with no URL decoding.
For example `patches/fix.patch` requires `SOURCES/fix.patch`, not `SOURCES/patches/fix.patch`.

The report lists each declaration, resolved path, byte size and observed SHA-256.
Missing files, non-regular files (including symlinks), mismatched declared digests, and different declared locations targeting one filename fail.
Identical resolved declarations may reuse a file.
Missing declared SHA-256 remains an authoring warning for remote Sources; computing local bytes does not satisfy `--policy submit` or modify the declaration.
Unknown macros, includes, `%sourcelist` and `%patchlist` are not executed or guessed and cannot establish a complete inventory.

TOML `materials` contains `valid`, `source_dir`, `files` and any inventory-level `error`; each file has a status and structured error when unavailable.
Top-level `valid` combines static and material results; `evidence.status` describes static checks only.
Ordinary `check` does not read materials.
Material checks collect independent file failures even when static rules fail.

This is a local snapshot, not a build certificate: no downloading, patch application, external `%files -f` evaluation, unused-file scan or native build.
Use a stable, trusted staging directory; mutation checks are best effort, not filesystem locking.
Recheck after changing inputs and before queuing a build.

## Static checks

`check`, generated candidates, and edit candidates share these checks:

| Rules | Scope |
| --- | --- |
| RPM010–RPM015 | Selected required main-package tags |
| RPK001 | SPDX expressions in package License tags and recognized top-level SPEC file-license comments; these are different licenses |
| RPK002 | Literal Name, Version, Release syntax; `Epoch: 0` is not rejected |
| RPK003 | Literal project URL syntax; existing HTTP/HTTPS accepted |
| RPK004 | Non-blocking advice to declare BuildSystem tool dependencies explicitly; static declarations only, not observed tool usage or dependency resolution |
| RPK005 | Missing or malformed adjacent Source SHA-256; warning for authoring, error under the submit static policy |

SPDX covers main/subpackages and conditional branches, not upstream license
correctness. IDs are case-insensitive, operators uppercase, deprecated IDs valid;
unknown IDs use bundled SPDX data. Missing file-license comments are not rejected
by this expression check; script comments are not declarations. Unresolved
license expressions leave checks incomplete even when a finding also proves failure.
Unchanged, confirmed main-package literal violations may remain during local editing; changed invalid values still fail admission; macro values are
not evaluated or certified.

RPK004 suggests explicit Autotools tool declarations; it does not block the operation.
Conditions, rich dependencies or expressions can make these suggestions uncertain.
The report records that uncertainty.
CMake/Meson have no shared tool requirement list here.
Unknown or context-dependent BuildSystem selections are not inferred.

`pass` means only the selected rules passed; parser warnings may remain. `check --policy authoring` (the default, also used by gen/edit) warns about missing Source digests. `check --policy submit` rejects confirmed digest violations and requires unambiguous static Source resolution. `-D 'MACRO EXPR'` supplies Source context and is recorded in the report; it does not evaluate License or other rules.
Unknown Source context is reported separately, not called a missing digest; it does not block unrelated authoring edits.
Nonblocking Source uncertainty remains in TOML evidence rather than producing repeated warnings for ordinary native build macros; submit explains it as incomplete.
Both policies are **static checks only**, not submission or release approval: source-content verification, native RPM validation and builds remain unperformed.
Use `source verify` separately to compare downloaded bytes.
Neither policy fully validates Source URLs or refreshes archive digests.
Neither check policy downloads sources, expands native RPM macros, resolves dependencies, verifies patches, or builds packages.

## Source downloads and static resolution

Source commands use a shared Rust HTTP/TLS client: no curl, RPM installation or container is required.
The client hashes streamed response bytes without HTTP content decoding.
Named hash operations can retain verified bytes in the WORK source cache; verification does not save archives.
HTTP(S) only; credentials in URLs, HTTPS-to-HTTP redirects, partial responses and truncated bodies are rejected.
Redirects are limited to 10 per attempt, connection setup to 10 seconds, and each source to a 300-second budget shared by redirects, body reads and retries.
Transient network failures and HTTP 408/429/502/503/504 get at most one retry after a 100 ms pause; TLS, URL-policy and other HTTP errors do not retry.
A retry restarts the hash, never resumes a partial digest.
Static macro evaluation is limited to 64 levels, 1 MiB per expanded value and 100,000 evaluation steps per context.
The `#/filename` suffix names the archive locally and is not sent to the server.
Proxy environment variables are supported.
TLS uses bundled Mozilla roots; `SSL_CERT_FILE` selects an explicit PEM CA bundle instead.
Certificate validation is never disabled.
Downloading a hash does not authenticate an upstream publisher.

SPEC input is parsed with `rpm-spec`.
Source identities, URL expressions and adjacent RemoteAsset declarations come from the same AST.
Static resolution supports:

- Known package tags and plain or braced references.
- Ordered nonparametric `%global` (eager), `%define` (lazy) and `%undefine` (pop).
- Conditional references with known presence.
- Integer and boolean conditions, and quoted-string equality.

Architecture/OS conditions need explicit `_target_cpu`/`_target_os`; the host is not the target environment. `-D 'NAME EXPR'` supplies definitions before reading the SPEC; later definitions in the SPEC can override them.
Cycles, excessive expansion and missing values produce reasons, not guessed URLs.
Unknown environment macros are **not** assumed undefined, including in `%{?name}`.
No Shell, Lua, parameterized macro or include is executed.
Generated declarations, `%sourcelist`, and Sources inside subpackages are unsupported.

## Verifying declared Source digests

```sh
ruyipack source verify --manifest package.toml --format toml
ruyipack source verify --spec package.spec -D 'archive_version 2.0'
```

Verification downloads every resolvable remote Source, including those with a SHA-256.
It never fills or replaces a checksum, creates a SPEC, or saves archives in the workspace.
Results are `match`, `mismatch`, `missing`, `unresolved`, or `error`; local materials are `not-applicable`.
Patch contents are outside this check.
When processing one input, gen, edit and verification share successful downloads for identical URL strings; every Source keeps its own declaration and result.
Failed downloads and later commands do not reuse this observation.
Missing declarations stay `missing` even when downloaded hashes are available; hexadecimal case does not affect comparison.
Failed downloads do not suppress later Sources.
Uncertain Source identities prevent enumerating the input rather than silently dropping a declaration.

Exit 0 means every applicable Source matched; missing, mismatch, uncertainty or failure exits 1.
TOML includes input identity, definitions, original expressions, separate `declared_sha256` and successful `download` evidence (SHA-256, URLs, byte count).
A changed input fails verification; retained results describe the recorded input hash, not the new file.
CLI errors exit 2 and may precede TOML.

## Computing or completing a Source digest

```sh
ruyipack source hash --spec package.spec --source 0 --format toml
ruyipack edit --spec package.spec --set package.version=2.0 --hash-source 0 --diff
ruyipack edit --spec package.spec --hash-source 0 --prepare drafts
```

`source hash` downloads one selected Source without changing recipe declarations.
Named WORK operations retain source bytes for reuse.
Human output writes the digest to stdout and preview/apply commands to stderr.
Those commands include an input-hash guard against stale edits.
TOML failures contain an error, not a fabricated hash.

`edit --hash-source N` calculates against the pending candidate, including version or URL edits.
All selected URLs resolve before downloading; all normal candidate and stale-input checks still run before publication.
Saved drafts must already select those digest fields.
Bare adjacent RemoteAsset markers can gain a digest; unmarked/local or ambiguous edit mappings are refused.
Failure leaves SPECs unchanged.
Macro-bearing digest markers must be repaired before calculation, so replacing one cannot change the meaning of the URL whose bytes were hashed.

Ordinary checking and editing stay offline; `gen` automatically attempts missing hashes unless `--offline` is used.
Default generation preserves existing digests; explicit `--hash` refreshes them in the candidate.
Author TOML remains unchanged.
No Source operation proves patch applicability or package build success.

## TOML and inspection

`inspect`, `check`, checked `gen`, `edit`, `source hash/verify`, build outcomes, and clean results use `--format toml` for machine reports; `json` is not an alias for these commands.
Optional unobserved values are omitted rather than represented by null.
Numbered source observations are arrays of records with an explicit `number` field.
Build outcomes link the full saved `receipt.json`, rather than re-encoding it.
Authoring schemas use JSON Schema.
Saved build, backend, engine, host and native-gate receipts use JSON.
Docker transport also uses JSON.
The native gate writes progress to stderr.

`inspect --format toml` returns main-preamble tag/conditional records, input SHA-256, parser identity, and diagnostics, not macro definitions or section bodies.
It does not evaluate conditions.
Recoverable parser errors do not fail inspection; use `check` for gates.
Human inspection normalizes tags and displays their conditional structure.

`inspect` record `span` fields refer to original UTF-8 bytes: zero-based, end-exclusive offsets; one-based lines/byte columns.
Verify the input digest before using them.
They are parser locations, not safe replacement ranges.
Invalid diagnostic byte bounds/order/UTF-8 boundaries and known unreliable macro coordinates omit unreliable spans without hiding messages.
This does not establish general semantic accuracy. `preamble` records preserve tag names, explicit material numbers, raw source slices and conditional branches.
They are not a round-trip parser AST.
Expression syntax is still tied to the recorded parser revision.
Report v2 identifies its `main-package-syntax` scope and does not check macro expansion, native RPM, or builds.

| Report | Contract |
| --- | --- |
| `check --format toml` | Report v2: input identity, parser, selected rules, `spec-static` evidence, findings and `parser_diagnostics` |
| `gen WORK --format toml` | Envelope v5, `manifest-generation-static` or `selected-generation-static`: input/profile/selected build-contract hashes and candidate report (including warnings) |
| `edit ... --format toml` | Envelope v5: `operation` is `edit`, `diff`, `check`, or `apply`; original/candidate identities, reports, draft paths or publication outcomes |

Static reports carry `evidence.incomplete_reasons` as a deterministic, deduplicated list, including when `status` is `fail`.
Reasons distinguish `parser-error`, `unresolved-license`, `unresolved-build-requirements`, and (for submit) `unresolved-sources`; an empty list does not expand the selected rule scope.
Parser errors stop rule execution.
Ordinary parser warnings do not imply incomplete checks.
Both generation and editing embed this same versioned report, independently of their outer envelope version.

`gen --check` writes no generated files.
Explicit publication targets still undergo path-safety checks. `--format` selects the operation report for generation, checking, diff or publication; it conflicts with raw `--stdout`.
TOML diff reports contain the diff as a field. `written` lists published SPECs; `artifacts` lists completed derived outputs.
The outer `input` identifies the consumed TOML; the nested candidate report names the intended SPEC destination, not an existing-file claim. `build_contract` is omitted in source-bound mode.
Profile/contract hashes identify embedded TOML, not an entire environment or all validation code.
With `--format toml`, input, rendering and static failures yield one report and exit 1.
Before a candidate exists, `report` is omitted and `error` explains the failure; unreadable input has no SHA-256.
CLI argument and output-write failures can precede TOML.

Static evidence records `policy`, `source_uncertainty` (omitted when resolved), and `not_checked` stages.
RPK005 uses the same ordered Source resolver as source hashing: known macros and conditions are evaluated without executing RPM macros; unknown declarations remain explicit.
Gen/edit retain authoring warnings for missing SHA-256.
A declared digest is never evidence that downloaded bytes match.

Reporting commands use an `error` object with `code` and `message`.
Unreadable/invalid-UTF-8 inputs report `valid = false`, `code = "input-read"`, and no fabricated input hash. `source hash` uses report version 2; `source verify` uses version 1.
Generation and edit envelope versions are listed above.
Codes describe the failing boundary, not text matched from a message.
Machine stdout is one complete TOML document, not line-delimited records; parse it with a TOML reader (for example Python 3.11+ `tomllib`).
Checked edit reports include the baseline; no separate original check is needed for that comparison.
Preparation (`scope: "edit"`) returns absolute draft paths, not a validated candidate.
Edit report version 5 separates `success` (the requested operation completed) from `valid` (all candidates passed the selected static checks).
Each candidate also reports `admissible`: whether the local edit may be saved.
Unchecked staging/diff has omitted `valid` and `admissible` fields, not a fabricated pass.
A successful edit may retain confirmed, unchanged authoring violations; it is not a passing package check.
Publication `outcomes` report `written`, `unchanged`, or `skipped`, actual destination paths, and known resulting hashes.
On partial I/O failure, `success` is false and `written` lists confirmed writes; remaining files are not claimed complete.
A missing receipt does not prove no write occurred.
If stdout fails after publication, stderr names confirmed writes when it is still writable; publication is not undone.
Neither channel is guaranteed when both are closed.
Successful in-place application advances the saved baseline; a failed baseline update is reported separately from any confirmed SPEC write.
Old immutable baselines remain available for recovery.
If SPEC publication succeeds but baseline update fails, fix the reported storage problem.
Then retry `edit --from STAGE --apply`.
Only an exactly matching, revalidated candidate can advance the baseline without rewriting the SPEC.

For edit, `baseline_report` records the original checks; `introduced_static_blockers` compares blocking rule facts, ignoring shifted positions (omitted when unresolved checks prevent attribution).
The editor shows its fixed selection before editing.
Publication may retain a known authoring violation only when that rule’s complete inputs are confirmed unchanged.
Changed invalid values, new blockers, incomplete checks and violations without input evidence still block. `edit --check` tests this same local-edit admission; standalone `check` and the submit policy remain strict.
For edit, `original_sha256` identifies the source; the nested input hash identifies the candidate.
Each file has `state = "pending-edit"` or `"candidate"`.
Paths have no human presentation suffix.
Errors have `code`, `message`, and optional `path`/`selected_fields`; selection is operation scope, not necessarily the offending field.
Codes include `unmappable-fields`, `source-changed`, `source-hash-failed`, `invalid-draft`, `invalid-assignment`, `invalid-candidate`, `static-check-failed`, and the fallback `operation-failed`.
CLI argument errors can precede TOML.
Diagnostics use lowercase severity names and remain inside TOML rather than stderr.

Pin tool/parser identities and check `format_version`.
Incompatible report changes increment it; optional fields and unknown error codes must be tolerated.
Saved drafts and manifests have no cross-version compatibility guarantee in this preview.

The native development gate limits commands to 120 seconds and terminates their process groups on timeout.
Source operations do not use it as a runtime fallback.

### Machine reports

A safely constructed `edit --diff` remains inspectable when an explicitly requested check fails; its exit status is 1.
Without `--check`, a diff is unchecked, not a claim that the package passes static rules. `--apply` always requires local-edit admission.
A construction or source-freshness failure produces no diff.

Publication errors retain their code/message and add `stage`, `reason`, and a path when known; I/O failures also carry `io_kind`.
On partial publication, `written` is authoritative: a failure is not a rollback.

Source failures carry `stage`, `reason`, `retryable`, and `http_status` when applicable. `retryable` concerns the download only, never the whole edit/apply.
Generation exposes best-effort failures in `source_hash_failures`; missing SHA-256 remains a warning, not a publication policy.

TOML reports include the producer's name, version, full Git `revision`, and `dirty` (tracked changes only, including staged changes).
Static check reports keep this under `evidence.tool`; other command envelopes use `tool`, including input failures.
Unknown revision/dirty fields are omitted, not a clean-tree claim.
Git is consulted only at build time and is optional; an archive never borrows an ancestor repository's revision.
Archive packagers can set `RUYIPACK_SOURCE_REVISION` to a full object ID and optionally `RUYIPACK_SOURCE_DIRTY=true|false` during the build.
These are supplied provenance, not independently verified claims or proof of a reproducible build.

### Repair package metadata

`check WORK --auto-fix` creates the package development area if needed, downloads
remote Sources that have no declared SHA-256, removes trailing periods from safely
mapped literal main-package Summary values, and publishes the checked repair.
It preserves existing digests; use `source verify WORK` to compare them with current
remote bytes. It does not upgrade versions, build or commit.
An unreferenced `.asc`, `.sig` or `.sign` Source1 is removed with its adjacent `#!RemoteAsset` comment
and matching file in the package directory. References, ambiguous material identity,
or other remaining signature paths prevent this removal. Cached downloads are not
package files; undeclared cached materials are not submitted to OBS.
Resolve pending TOML edits first. Download failure leaves the recipe unchanged.
`--format toml` returns one repair report with creation, changes, downloads and
publication outcomes. The report also includes the candidate static checks;
remaining warnings are not a claim that every package issue was repaired.

### Check for upgrades

`ruyipack check WORK --upgrade` runs static checks and queries Repology without
changing the recipe. The TOML report includes both findings and an `upgrade` table.
Version availability and lookup failures are advisory; the exit status follows the
static policy and, if requested, material checks. Plain `check` remains offline.
`ruyipack check WORK --upgrade --auto-fix` applies a known newer version and computes
missing digests plus digests whose Source URL changes. Existing digests for unchanged
URLs are retained. Pending TOML edits must be resolved first. A failed download does
not publish the candidate SPEC. Neither command proves patch or build compatibility.

The default project is the lowercase SPEC Name. `python3-` and `python-` map to
`python:`. Override with `--repology-project PROJECT`, or add explicit mappings in
`.ruyiconfig/config.toml`:

```toml
[repology]
python3-example = "python:example"
```

Unknown current versions, ambiguous newest versions, and non-newer RPM versions are
reported, not applied. Network failures are not treated as “already current”.
Upgrade lookup failure does not block independent basic repairs. Packages with
Patch declarations require manual upgrade review; their basic repairs can still run.
`check --auto-fix` repairs missing Source digests and literal main Summary punctuation;
It also replaces a sole `%{?autochangelog}` or `%{autochangelog}` body with
`%autochangelog`, and fixes spacing in parsed BuildRequires, BuildOption and
%files headers. Manual changelog entries and comments are preserved.
Directory renaming, arbitrary release expressions and missing changelog sections
are not repaired. A successful repair is not a claim that repository hooks or
build checks passed; run those checks on the resulting files.

## Pull requests

Commit package changes before you run `task pr`. Each commit must change one package.
A plan can group many package commits into one PR without squashing them.
The same plan can be used by `task remote-build`; `[pr]` does not change build settings.

```toml
[[packages]]
work = "ed"

[pr]
title = "Update ed"
base = "main"
# target = "OWNER/REPO"  # Default: the recipe repository's origin.
# template = ".ruyiconfig/pr.md"  # Relative to this plan.
```

```sh
ruyipack task --plan packages.toml pr
ruyipack task --plan packages.toml pr --template body.md --format toml
ruyipack task --plan packages.toml pr --publish
```

Preview is offline. It checks a clean topic branch, every commit's package scope,
and exact WORK file identities against the committed tree. It refuses unrelated
changes, merge commits, duplicate package bindings and uncommitted WORK changes.
`--repo PATH` selects a prepared local recipe repository instead of the configured path.
The local base branch must exist and be an ancestor of the topic branch.

`init` writes the embedded openRuyi template to `.ruyiconfig/pr.md`.
Edit that file to set workspace defaults. `work/NAME/pr.md` overrides it for
one WORK. For a multi-WORK PR, effective templates must match; otherwise use
`--template` to select one. Explicit `--template`, then `[pr].template`, override
WORK and workspace defaults. Repeated `init` and program upgrades do not replace
these user-owned files. Existing workspaces must add this file before using the
default path.

The body template must contain `{{summary}}` and `{{obs_links}}` once each.
Summary entries come from commit actions, not a second list in the plan.
`commit --action` preserves each action in an `Action:` Git trailer. For commits
without these trailers, the `SPECS: PACKAGE: ACTIONS` subject uses comma-separated
actions and a final ` and `. Use trailers when one action contains these delimiters.
Repeated actions are listed once. Only actions confined to one package get a package
prefix. Package notes appear below that package's actions; notes with no unique action
get a package heading. These entries describe changes, not test results.
OBS project links come from matching remote-build receipts. A missing or stale receipt
produces a warning, not a build-success claim. The default template puts both fields
under Summary. Add the contribution checklist required by the target project to your
template; do not mark policy acceptance automatically.

Publication requires authenticated `gh` and Git push access to origin. It checks the
remote base, pushes without force, and creates a draft PR. A matching open PR is updated;
a closed PR is not reopened. It does not create commits, rewrite history, fetch, or build.
A TOML report includes `head`, `base_revision`, commit IDs, the generated body, `stage`,
`pushed`, `published` and the PR URL. If PR creation fails after push, `pushed` remains true.
Retry checks for an open PR before creating another. Review network failures before retrying.

`commit --action TEXT` can be repeated for a structured action list. Generated subjects
use `SPECS: PACKAGE: first, second and last`. `--message` remains a verbatim override
and cannot be combined with `--action`.

Basic auto-fix removes unreferenced verification Sources (`.asc`, `.sig`, `.sign`,
`.pub`), including download URLs with a renamed local file. It removes their adjacent
RemoteAsset markers and owned package files. Remaining Source numbers and literal
`SOURCE` macro references are changed together. Source0 is retained for implicit setup.
Opaque numbering, conditional declarations, computed references and bulk material
access prevent automatic cleanup. Ordinary unreferenced data is not treated as a signature.

### Configure scripts

Run `ruyipack edit WORK --menu` and choose `conf`, then `-p`, `-a`, or
`replace`. Enter the script text. `-p` runs before the default configure
script; `-a` runs after it. `replace` replaces only the main `%conf` body,
not its prepend or append fragments. These edits use the same candidate
checks and `--apply` gate as metadata edits.

For a noninteractive edit:

```sh
ruyipack edit WORK --set 'build.stages.conf.prepend=echo configuring' --diff --apply
```

`--set` replaces a fragment. `--add` appends text after its existing body,
with a newline separator:

```sh
ruyipack edit WORK --add 'build.stages.conf.prepend=echo another-step' --diff --apply
```

An empty fragment receives the text without a leading blank line. Repeating
`--add` appends again; it is not an idempotent operation. Do not combine
`--set` and `--add` for the same field. Conditional or duplicate configure fragments are not
silently merged. Scripts execute during the build, not during editing.

`task --plan packages.toml pr --close` closes the matching PR and cancels its unfinished
Actions runs. It does not delete branches. The report separates PR closure,
cancellation requests, and confirmed finished runs. Other PRs' runs are not cancelled.
Run `--publish` after closure to create a replacement draft PR.

### Require local build evidence before committing

`ruyipack commit WORK --require-build` requires a successful full local build
whose SPEC and material inputs match the proposed delivery. Missing, failed,
prep-only or stale results block publication. With `--dry-run`, the command
still prints the proposed diff and returns failure when this requirement is not met.

This checks the retained local build, not OBS results or repository CI checks.
The build's recorded environment and release policy still define its scope;
a development macro configuration does not validate OBS release services.
Without this option, manual commits retain their existing static and material checks.

## Check a prepared directory

```sh
ruyipack check --directory /path/to/prepared --check-output /path/to/new-result --format toml
```

The directory supplies `scripts/remoteassetify.py` and `SPECS/`. All SPEC files
under `SPECS/` are checked, including uncommitted files. Prepare only the package
scope you want to check. No Git repository, commit or remote address is required.
Review the script before running it. Inputs are copied before execution.

This check downloads materials. It does not build packages or run all GitHub
Actions checks. Ordinary `check WORK` remains offline.

The embedded Fedora environment uses the Docker daemon's native platform and
runs without Mock. Use `--check-config FILE` for a Compose environment with
service `worker`, Python 3.11 or newer, rpm, curl and enosys. The worker reads
`/input` and writes `/output`. `--context` selects the Docker connection;
`--timeout` limits execution time.

A new result directory stores `receipt.toml`, input and script hashes, image
identity, commands, and check results. Backend logs are in `host/`; check logs
are in `engine/`. Incomplete checks fail. Completed results are retained.
The worker is removed after collection. Empty input does not start Docker.
Fedora results do not establish Ubuntu runner equivalence.
