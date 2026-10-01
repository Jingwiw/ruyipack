<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# Command and manifest reference

[Quick start](../README.md#try-it) · [Design and policy sources](design.md)

This reference covers semantics beyond `ruyipack COMMAND --help`.

## Workspace initialization

`ruyipack init [PATH] [--clone URL]` creates `.ruyiconfig/config.toml` and the
embedded openRuyi build configuration in an empty directory (the current directory
by default). Without `--clone`, initialization needs no Git, Docker, or network
access. Initialization never starts Docker or creates a package. Existing markers
cause a warning, including interrupted or damaged initialization; files are never
repaired or overwritten implicitly.

The configuration uses three paths: `recipes` defaults to `openruyi` and may be
external; `work` defaults to `work` and stays inside the workspace; `specs` defaults
to `SPECS` relative to the recipe repository. Re-running bare `init` diagnoses
invalid base configuration without requiring the recipe repository or build tools.

`--clone URL` first completes initialization, then runs Git clone into the configured
`recipes` path. Git or network failure exits 1 while retaining the initialized
configuration and any files Git left behind; the diagnostic distinguishes successful
initialization from failed cloning. Re-running bare `init` never clones. Explicit
`--clone` can retry an already valid workspace only while its configured recipe
destination is absent. Any existing destination, even an empty or interrupted
directory, is refused without overwrite; inspect retained files before retrying.
An invalid workspace configuration blocks cloning and is not repaired.
Git clone has a 300-second execution budget; other Git calls have 30 seconds.
User Git configuration, filters, and SSH/credential helpers remain trusted;
this is not a sandbox.

## New development area and scaffold

`new WORK [--pkgname PKG]` discovers the nearest `.ruyiconfig` above the current
directory. Prepare its configured recipe Git repository with a committed `main`
first. PKG defaults to WORK; `new ed-test --pkgname ed` creates:

```text
work/ed-test/
  .config.toml                 # saved pkg = "ed" binding
  ed.toml                      # author input, outside Git
  checkout/                    # one linked Git worktree
    SPECS/ed/
```

The checkout starts at the recipe repository's exact committed `main` on a separate
branch. It preserves non-SPECS directories and selects only the bound package
under SPECS; shared files directly under SPECS may also remain. Uncommitted files
in the selected package block creation. Unrelated dirty recipe files stay untouched
and are not copied. A package absent from that commit requires explicit `--pkgname PKG` for a new scaffold;
the scaffold header explains that existing SPEC contents are not imported.
Git uses trusted repository/user configuration, including checkout filters; this is
not a sandbox, and configured filters may perform file or network I/O.

The saved binding is reused by `new WORK`. A conflicting `--pkgname` is rejected even
with `--force`; repeating `new` never resets the checkout or changes its branch.
Interrupted areas are diagnosed rather than overwritten. `--stdout` and `--diff`
perform preflight but create no area, binding or Git worktree. TOML output follows
the [shared output rules](#output-and-file-safety), including preservation of manual
content unless `--force` explicitly replaces the scaffold.

No build system is selected by default. `--build-system autotools|cmake|meson`
adds stage guidance; `--comments full` adds explanations and a commented
subpackage example without changing values. Autotools prefills `autoconf`,
`automake`, `libtool`, and `make`. CMake/Meson do not prefill a common tool set.
Declare actual dependencies and confirm macros in the target environment.

The scaffold saves the current year and the configured Git author from the recipe
repository (or existing checkout). An unavailable or unsuitable author leaves an
empty contributor list and a warning. Fill in package facts, sources, dependencies,
stages and files; `gen` uses saved values, not the current identity or date.
Use `gen WORK` to read its saved `PKG.toml` and explicitly generate
`checkout/SPECS/PKG/PKG.spec`. A new generation never imports an existing SPEC
back into TOML, and build never implicitly generates a recipe.

## Common SPEC selection

Human diagnostics identify the input once, then use `spec[LINE:COLUMN]` (or
`spec` when no location is known). TOML retains full paths and source positions.

A positional argument always selects WORK. The first implicit package lookup
requires committed `SPECS/WORK/WORK.spec`; `--pkgname PKG` explicitly binds another
package. Without a match or explicit binding, the command fails before allocation.
For existing WORK, the saved binding wins and conflicting `--pkgname` fails.

`inspect`, `check`, `schema edit`, and `source` create only WORK's binding when
checkout is absent, then read the recipe's committed main blob. Reports include
that commit as `input.revision`. Current recipe edits are not read or copied.
`edit WORK` creates checkout for an actual edit, but does not publish SPEC without
`--apply`. A check without editing reads existing stage or committed main. Read-only operations
may create the small WORK state; they do not create a branch or publish SPEC.
Each later operation uses current main until checkout exists, not a hidden sticky
snapshot. Use `edit --expect-sha256 HASH` to pin the bytes you reviewed.
Writes materialize checkout from committed main after preflight; selected dirty
recipe materials still block checkout creation. Existing checkout's current branch
and bytes are authoritative and are never reset.

Use `--spec PATH` for external files. This is explicit even for `.spec` filenames;
file existence never changes argument meaning. `check --manifest PATH` and
`source verify --manifest PATH` select authoring input instead. These modes are
mutually exclusive, not fallback searches.

## Manifest editor schema

`ruyipack schema manifest > ruyipack.schema.json` exports the authoring schema from the
installed binary, without reading a package, downloading anything or writing files
itself. Put this directive at the start of your manifest (followed by a blank line):

```toml
#:schema ./ruyipack.schema.json

[package]
# ...
```

[Tombi and compatible TOML editors](https://tombi-toml.github.io/tombi/docs/json-schema/)
use it for completion, field descriptions and structural diagnostics. Paths are
relative to the manifest. Regenerate the local schema after updating RuyiPack;
no mutable remote URL or network access is required. `new` does not create a
sidecar or reference a file that may not exist.

`schema edit WORK --field FIELD` instead describes the selected fields of an existing
SPEC. Authoring schema checks unknown/missing fields, types, supported build/stage
names, Source/Patch shapes and supplied SHA-256 syntax. Missing SHA-256 remains
allowed. RPM expressions, numeric aliases/overflow, required Source0, VCS choices,
file-list content and other cross-field constraints still require
`ruyipack gen WORK --offline --check`; schema success is not build validation.
Unfilled `new` scaffolds intentionally pass structural checks, not generation.

## Generate

`gen WORK` reads the input selected in WORK's binding: `authoring` reads
`work/WORK/PKG.toml`; `edit` reads the saved source-bound stage. File existence
and modification times do not choose the authority. `new` selects authoring
when its scaffold is actually present; changing the default WORK stage selects
edit. An external `--prepare DIR` does not change WORK's input.

Use `gen WORK --input authoring` or `--input edit` to select explicitly. A
successful normal generation saves that selection without deleting the other
input. `--check`, `--stdout` and `--diff` use an explicit selection only for that
invocation. Missing or stale selected inputs fail; there is no fallback to a
cached SPEC, resolved TOML, or another input.

By default, generation writes `PKG.resolved.toml` beside its input and a
candidate SPEC; author input and checkout remain unchanged. The snapshot is
read-only standard TOML (format version 2): `manifest` contains the validated values used by the
renderer, `profile` the target defaults, and `downloads` the actual observations.
It preserves RPM expressions rather than claiming native macro expansion.
Source-bound snapshots instead have an `edit` section with the immutable source
identity, selected fields and resolved values; unsupported scripts stay in SPEC.
Neither snapshot is an authoring manifest or an automatic input to `gen`.

`--spec=.` writes `PKG.spec` beside the snapshot; `--spec=auto` publishes the
checkout SPEC; `--spec=PATH` names a complete destination. Publications use
static admission (the same local-edit gate for source-bound input).
`--check` writes no snapshot, candidate or SPEC. `--stdout` and `--diff` print
candidate payloads without publication. Preview/check may still download unless
`--offline` is set. See [ed.toml](../examples/ed/ed.toml) for authoring input.

### Repository and Sources

Choose an entry in `[package.vcs]` only when the fact is known:

| Entry | Generated result |
| --- | --- |
| `git = "https://example.org/project.git"` | `VCS: git:...`; do not include the prefix in the input |
| `same-as-url = true` | Omit VCS; `package.url` already names the source repository |
| `no-public-repository = true` | Emit the profile's no-repository comment |

Omit the table or leave it empty while the repository status is unknown. Generation
warns (also in TOML `authoring_warnings`), emits no VCS assertion, and never turns
an unknown or failed lookup into `no-public-repository`. Conflicting choices fail.
Addresses and declarations are not verified remotely. A generated candidate is not
proof of VCS policy compliance; confirm the declaration before publishing.
The ed example illustrates an explicit declaration, not verified repository evidence.

Each `[sources.N]` chooses exactly one material form:
`url` with optional `sha256` for a remote resource, or `path` for a local input.
`[patches.N]` accepts a local `path` only. Local paths receive no RemoteAsset
marker or hash; gen does not read, copy, or confirm these files. URL/path combinations and hashes on local material fail.
Source and Patch numbers have independent namespaces. Sources are sorted numerically;
Patches retain TOML declaration order, because native `%autopatch` uses that order.
Patch declarations are placed after BuildSystem; application, strip level, and patch order remain the
responsibility of the prep stage/declarative RPM machinery. Follow openRuyi's
four-digit patch filename categories and document patch purpose in its header;
generation does not inspect patch content or test applicability.

For example:

```toml
[sources.1]
path = "example.service"
[patches.2000]
path = "2000-fix-build.patch"
```

`sources.0` is the primary archive used by the
Autotools default unpacking step. A missing digest produces a bare `#!RemoteAsset`
and warning, not an error. This output does not meet openRuyi's SHA-256 requirement
for HTTP(S) sources. An empty digest fails. Digests must be 64 hexadecimal digits;
case is preserved.

After filling the scaffold, `gen WORK` automatically attempts to download remote
Sources whose `sha256` is absent and fill the generated SPEC. Use `--offline` to
skip downloads entirely. Existing digests are retained, not downloaded or
verified; remove a digest from the TOML when you intend to recalculate it.
Completion uses built-in Rust HTTP/TLS and the package fields below, without
executing macros. Download failures warn per Source and leave its digest missing.
Connection setup is limited to 10 seconds; the complete transfer, including
redirects and reading the body, to 300 seconds.

Generation serializes the same validated values used for rendering into a
read-only resolved snapshot on a normal invocation. The author input remains unchanged. `--check --format
toml` includes successful downloads in `source_hashes` (an empty array with `--offline`),
failures in `source_hash_failures`, and contextual warnings in `authoring_warnings`.
Preview/check modes may download unless `--offline` is set. Explicit `--hash`
recalculates existing declarations; ordinary completion does not replace them.
A digest is an observation of downloaded bytes, not source authentication. If a
version/URL changes and its omitted digest cannot be refreshed, the result warns;
it does not certify the old declaration as a newly calculated fact.

Source expressions may use `%{name}`, `%{version}`, and `%{url}` only when the
referenced package values are unambiguous static literals. Expressions and
filename fragments such as `#/archive.tar.gz` remain in the SPEC. Literal percent
escapes use `%%` (for example `a%%20b.tar.gz`); other macro tokens are unsupported.
New manifests require HTTPS; editing permits existing HTTP Sources. Authored URLs
reject userinfo credentials, but this is not a secret scan or a redaction service:
query strings and existing text in views/diffs may contain secrets.

### Build stages

Stages are `prep`, `conf`, `build`, `install`, and `check`, emitted in that order.
With `build.system`, RPM supplies default actions; the following fields customize
one stage:

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

Nonempty `options` and `replace` cannot coexist in one stage. Put arguments in the
replacement instead. Hooks still surround a replaced action. Scripts require LF;
indentation, comments, and macros are preserved. Keep reasons for skipped tests
in script comments. Validation checks boundaries and text, not shell correctness.

Without `build.system`, supply explicit scripts, for example:

```toml
[build.stages.prep]
replace = '%autosetup -p1'
[build.stages.build]
replace = '%make_build'
[build.stages.install]
replace = '%make_install'
```

There are no default stages, including unpacking; omitting `[build]` emits none.
`options` requires a system. Declare tools in `[build-requires]`; no build-system
requirements are added or enforced in this explicit mode.

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

A key is a suffix (`devel` → `<main-name>-devel`); `full-name = true` makes it a
complete name and uses `-n` consistently in `%package`, `%description`, and
`%files`. Names must be literal RPM names with no resulting main/subpackage
collisions. Each subpackage needs nonempty summary and description. Requires and
Provides use the main package's expression rules; files accept `license`, `doc`,
`entries`, and `lists`. Empty/omitted subpackage files emit an empty `%files`
for a metapackage; the main package needs at least one entry or external list.

Per-subpackage license/URL/architecture, conditional declarations, and
macro-generated families are unsupported. File ownership still needs build-time
verification.

### Native file entries and generated lists

Main and subpackage `files` use the same fields. `license` and `doc` remain
shortcuts; ordered `entries` also accept native file rows such as `%dir`,
`%config(noreplace)`, `%ghost %attr(0644,root,root)`, `%verify`, `%exclude`, and
`%defattr`. Do not put conditions, sections, or arbitrary macro statements there.
The renderer preserves row order and the verifier compares directives and paths.
The `license`/`doc` shortcuts are emitted before `entries`. When order matters
(for example around `%defattr`), put all affected rows in `entries` instead.

```toml
[package.files]
license = ["COPYING"]
lists = ["%{name}.lang", "generated.files"]
entries = ["%dir %{_datadir}/example", "%config(noreplace) %{_sysconfdir}/example.conf"]
```

`lists` emits repeated `%files -f` arguments, without guessing the package name or
adding an implicit list. It can be the only content of a files section. These
lists are produced/read during the RPM build, never opened by gen. File existence,
macro expansion, final ownership, and list contents still need native build
validation. `%files -l` and conditional file sections are not supported.

## Edit

WORK selection follows [the common input rules](#common-spec-selection).
Use `edit --spec PATH` for independent files; repeat `--spec` for a batch.
`--from` retains its saved absolute SPEC paths. Named operations hold their WORK
lock through validation and publication; direct-file edits do not use that lock.

Default `edit WORK` opens a persistent TOML stage using `--editor`, `$VISUAL`,
`$EDITOR`, then `vim`. No menu, package static check or SPEC write happens implicitly.
GUI editors must wait, e.g. `--editor 'code --wait'`; editor output goes to stderr.
Unfinished TOML remains saved even when it cannot yet generate a candidate.

`--menu` selects fields and edits their current values inline. `--set FIELD=VALUE`
is non-interactive. Every editing path saves the same TOML document.
`--field FIELD` edits the current value directly in the terminal; use `--editor COMMAND`
for selected TOML editing, or `--prepare DIR` to save it without opening an editor.
`--all` requests strict full mapping.
`inspect WORK --editable --field FIELD` and `schema edit WORK --field FIELD` are
separate read-only projection/schema queries.

`--diff` constructs the same safe candidate as `gen`, caches the SPEC and saves
one `.diff` beside the stage TOML input (not the checkout SPEC); it also prints the diff (TOML reports its path).
`--check` additionally checks the edited candidate. `--apply` always checks local
admission and publishes only when admissible. These options can be combined.
`--hash` refreshes every remote HTTP(S) Source, including signatures;
`--hash-source N` selects individual Sources. Both use the edited candidate and
save the resulting facts back to the stage. Local materials/Patches are excluded.

The subset covers existing main-package metadata, Sources with adjacent
RemoteAsset markers, BuildSystem/BuildRequires, descriptions, simple file lists,
header metadata, comments, and changelog text. Full views require every construct
to be mapped; VCS tags and build scripts need a selected-field view. Unselected
bytes stay intact. Parser errors or ambiguous selected fields stop editing.
Deleting keys or adding unmapped groups fails; supported existing lists can change.
Appending BuildRequires preserves comment-separated groups; other nonempty size
changes require a contiguous group.

Unnumbered `Source:` uses its effective RPM number: after `Source3:`, it is 4,
not 0. Conditions, includes, or unsupported expressions can make implicit numbers
uncertain; affected Source edits fail rather than guess. Unrelated fields may
still be edited. Recognizable invalid URL/digest values can be viewed and repaired
if their ranges are unambiguous; selected replacements must validate. Unselected
digests are preserved, not certified. A bare RemoteAsset exposes an empty `sha256` value;
`--set sources.0.sha256=HASH` adds a validated digest to that same marker. An empty
value preserves a bare marker but cannot erase an existing digest.

Version or Source URL changes produce `review_triggers` and `review_required`
(source/digests, patches, native build). They do not turn a static pass into a
failure. Empty lists mean no triggering change, not that external checks ran.
Use `--expect-sha256 HASH` for a single-file edit based on an earlier read or
`source hash` result. A mismatch refuses the operation; publication still checks
for subsequent changes. This binds the SPEC bytes, not external macros or URLs.

### Persistent drafts

```sh
ruyipack edit --spec ed.spec --spec other.spec --field package.version --prepare drafts
# Edit the TOML files in drafts, then:
ruyipack edit --from drafts --check --format toml
ruyipack edit --from drafts --diff
ruyipack edit --from drafts --apply
```

`--from` resumes without an editor; choose `--check`, `--diff` or `--apply`, or
add `--editor COMMAND` to reopen TOML.
Drafts use source basenames and TOML 1.1. Keep `.state` (original bytes, source
identities, and schemas) with the editable files. Duplicate basenames need separate
directories. A successful in-place apply advances the saved baseline. External source changes
require a fresh stage; `--force` never overrides that boundary.
Editor work is retained after validation failure or when changes remain unapplied.

## Output and file safety

- `gen` defaults to resolved TOML and a cached SPEC; `new` writes `PKG.toml` inside
  the selected development area.
  `edit` defaults to persistent staging, and requires `--apply` to replace SPEC. Explicit output paths are relative to
  the current directory; parent directories must exist.
- `--stdout` prints one candidate without consulting the target. `--diff` prints
  a unified diff without publishing SPEC (edit also saves candidate/diff); edits can preview a
  batch. Both return 0 on successful previews regardless of differences. Diff
  headers require UTF-8 paths without tabs/newlines.
- Different existing outputs require an explicit action or terminal confirmation.
  `--force` replaces; `--skip-existing` keeps existing files and creates missing
  ones. These apply to new/gen; edit's `--force` requires `--output`. Same-content
  writes leave files untouched. See `--help` for incompatible option combinations.
- Without a usable terminal, unresolved conflicts fail. The terminal menu offers
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

`--materials` adds offline Source/Patch checks to the normal static checks; it
never replaces them. Manifest input is rendered and verified in memory, without
hash downloads or output files. Diagnostic positions then refer to the generated
SPEC; TOML identifies both the original input and generated SPEC digest.

`--source-dir` requires `--materials`. It selects the prepared RPM `_sourcedir`;
by default this is the canonical recipe's directory, **not** the working directory.
Both local declarations and downloaded URL resources must already be staged there.
The filename follows RPM rules: the suffix after the last `/`, then after the last
`=` in that suffix, with no URL decoding. For example `patches/fix.patch` requires
`SOURCES/fix.patch`, not `SOURCES/patches/fix.patch`.

The report lists each declaration, resolved path, byte size and observed SHA-256.
Missing files, non-regular files (including symlinks), mismatched declared digests,
and different declared locations targeting one filename fail. Identical resolved
declarations may reuse a file. Missing declared SHA-256 remains an authoring
warning for remote Sources; computing local bytes does not satisfy `--policy submit`
or modify the declaration. Unknown macros, includes, `%sourcelist` and `%patchlist`
are not executed or guessed and cannot establish a complete inventory.

TOML `materials` contains `valid`, `source_dir`, `files` and any inventory-level
`error`; each file has a status and structured error when unavailable. Top-level
`valid` combines static and material results; `evidence.status` describes static
checks only. Ordinary `check` does not read materials. Material checks collect
independent file failures even when static rules fail.

This is a local snapshot, not a build certificate: no downloading, patch
application, external `%files -f` evaluation, unused-file scan or native build.
Use a stable, trusted staging directory; mutation checks are best effort, not
filesystem locking. Recheck after changing inputs and before queuing a build.

## Static checks

`check`, generated candidates, and edit candidates share these checks:

| Rules | Scope |
| --- | --- |
| RPM010–RPM015 | Selected required main-package tags |
| RPK001 | SPDX expressions in package License tags and recognized top-level SPEC file-license comments; these are different licenses |
| RPK002 | Literal Name, Version, Release syntax; `Epoch: 0` is not rejected |
| RPK003 | Literal project URL syntax; existing HTTP/HTTPS accepted |
| RPK004 | openRuyi declared BuildSystem contract versus direct BuildRequires; not observed tool usage or dependency resolution |
| RPK005 | Missing or malformed adjacent Source SHA-256; warning for authoring, error under the submit static policy |

SPDX covers main/subpackages and conditional branches, not upstream license
correctness. IDs are case-insensitive, operators uppercase, deprecated IDs valid;
unknown IDs use bundled SPDX data. Missing file-license comments are not rejected
by this expression check; script comments are not declarations. Unresolved
license expressions leave checks incomplete even when a finding also proves failure.
Unchanged, confirmed main-package literal violations may remain during local editing; changed invalid values still fail admission; macro values are
not evaluated or certified.

Missing Autotools tools fail unless unresolved conditions, rich dependencies, or
expressions could supply them, in which case evidence is incomplete. CMake/Meson
have no common required-tool set here. Unknown/context-dependent BuildSystem
selections are outside this contract, not inferred.

`pass` means only the selected rules passed; parser warnings may remain.
`check --policy authoring` (the default, also used by gen/edit) warns about missing
Source digests. `check --policy submit` rejects confirmed digest violations and
requires unambiguous static Source resolution. `-D 'MACRO EXPR'` supplies Source
context and is recorded in the report; it does not evaluate License or other rules.
Unknown Source context is reported
separately, not called a missing digest; it does not block unrelated authoring edits.
Nonblocking Source uncertainty remains in TOML evidence rather than producing
repeated warnings for ordinary native build macros; submit explains it as incomplete.
Both policies are **static checks only**, not submission or release approval:
source-content verification, native RPM validation and builds remain unperformed.
Use `source verify` separately to compare downloaded bytes. Neither policy fully
validates Source URLs or refreshes archive digests. Neither check policy downloads sources, expands native RPM macros, resolves
dependencies, verifies patches, or builds packages.

## Source downloads and static resolution

Source commands use a shared Rust HTTP/TLS client: no curl, RPM installation or
container is required. Responses stream directly into SHA-256 without temporary
archives or HTTP content decoding. HTTP(S) only; credentials in URLs, HTTPS-to-HTTP
redirects, partial responses and truncated bodies are rejected. Redirects are
limited to 10 per attempt, connection setup to 10 seconds, and each source to a
300-second budget shared by redirects, body reads and retries. Transient network
failures and HTTP 408/429/502/503/504 get at most one retry after a 100 ms pause;
TLS, URL-policy and other HTTP errors do not retry. A retry restarts the hash,
never resumes a partial digest. Static macro evaluation is limited to 64 levels,
1 MiB per expanded value and 100,000 evaluation steps per context.
The `#/filename` suffix names the archive locally and is not sent to the server.
Proxy environment variables are supported. TLS uses bundled Mozilla roots;
`SSL_CERT_FILE` selects an explicit PEM CA bundle instead. Certificate validation
is never disabled. Downloading a hash does not authenticate an upstream publisher.

SPEC input is parsed with `rpm-spec`. Source identities, URL expressions and
adjacent RemoteAsset declarations come from the same AST. Static resolution supports
known package tags, plain/braced references, ordered nonparametric `%global`
(eager), `%define` (lazy), `%undefine` (pop), known-presence conditional references,
integer/boolean conditions and quoted-string equality. Architecture/OS conditions
need explicit `_target_cpu`/`_target_os`; the host is not the target environment.
`-D 'NAME EXPR'` supplies definitions before reading the SPEC; later definitions
in the SPEC can override them. Cycles, excessive expansion and missing values
produce reasons, not guessed URLs. Unknown environment macros are **not** assumed
undefined, including in `%{?name}`. No Shell, Lua, parameterized macro or include
is executed. Generated declarations, `%sourcelist`, and Sources inside subpackages are unsupported.

## Verifying declared Source digests

```sh
ruyipack source verify --manifest package.toml --format toml
ruyipack source verify --spec package.spec -D 'archive_version 2.0'
```

Verification downloads every resolvable remote Source, including those with a
SHA-256. It never fills or replaces a checksum, creates a SPEC, or saves archives
in the workspace. Results are `match`, `mismatch`, `missing`, `unresolved`, or
`error`; local materials are `not-applicable`. Patch contents are outside this check.
Missing declarations stay `missing` even when downloaded hashes are available;
hexadecimal case does not affect comparison. Failed downloads do not suppress
later Sources. Uncertain Source identities prevent enumerating the input rather
than silently dropping a declaration.

Exit 0 means every applicable Source matched; missing, mismatch, uncertainty or
failure exits 1. TOML includes input identity, definitions, original expressions,
separate `declared_sha256` and successful `download` evidence (SHA-256, URLs, byte
count). A changed input fails verification; retained results describe the recorded
input hash, not the new file. CLI errors exit 2 and may precede TOML.

## Computing or completing a Source digest

```sh
ruyipack source hash --spec package.spec --source 0 --format toml
ruyipack edit --spec package.spec --set package.version=2.0 --hash-source 0 --diff
ruyipack edit --spec package.spec --hash-source 0 --prepare drafts
```

`source hash` downloads one selected Source without writing. Human output is its
digest on stdout and copyable preview/apply commands on stderr, with an input hash
guard against stale edits. TOML failures contain an error, never a fabricated hash.

`edit --hash-source N` calculates against the pending candidate, including version
or URL edits. All selected URLs resolve before downloading; all normal candidate
and stale-input checks still run before publication. Saved drafts must already
select those digest fields. Bare adjacent RemoteAsset markers can gain a digest;
unmarked/local or ambiguous edit mappings are refused. Failure leaves SPECs unchanged.
Macro-bearing digest markers must be repaired before calculation, so replacing
one cannot change the meaning of the URL whose bytes were hashed.

Ordinary checking and editing stay offline; `gen` automatically attempts missing
hashes unless `--offline` is used. Existing gen digests and author TOMLs are never
updated. No Source operation proves patch applicability or package build success.

## TOML and inspection

`inspect`, `check`, checked `gen`, `edit`, `source hash/verify`, build outcomes, and clean results use
`--format toml` for machine reports; `json` is not an alias for these commands.
Optional unobserved values are omitted rather than represented by null. Numbered
source observations are arrays of records with an explicit `number` field.
Build outcomes link the full saved `receipt.json`, rather than re-encoding it.
Authoring schemas remain standard JSON Schema. Persistent build/backend/engine/host receipts
still use their existing JSON protocols; their TOML migration is not implemented
by this report change. Docker's JSON transport remains an external boundary. The standalone native gate's legacy structured JSON receipts are outside this CLI report migration; its human progress goes to stderr.

`inspect --format toml` returns main-preamble tag/conditional records, input SHA-256, parser identity,
and diagnostics, not macro definitions or section bodies. It does not evaluate
conditions. Recoverable parser errors do not fail inspection; use `check` for gates.
Human inspection normalizes tags and displays their conditional structure.

`inspect` record `span` fields refer to original UTF-8 bytes: zero-based, end-exclusive
offsets; one-based lines/byte columns. Verify the input digest before using them.
They are parser locations, not safe replacement ranges. Invalid diagnostic byte
bounds/order/UTF-8 boundaries and known unreliable macro coordinates omit
unreliable spans without hiding messages. This does not establish general semantic accuracy.
`preamble` records distinguish tag names and explicit Source/Patch numbers, preserve raw source slices and conditional branches, and do not form a round-trip parser AST. Expression syntax is still tied to the recorded parser revision. Report v2 identifies its `main-package-syntax` scope and does not check macro expansion, native RPM, or builds.

| Report | Contract |
| --- | --- |
| `check --format toml` | Report v2: input identity, parser, selected rules, `spec-static` evidence, findings and `parser_diagnostics` |
| `gen WORK --check --format toml` | Envelope v4, `manifest-generation-static` or `selected-generation-static`: input/profile/selected build-contract hashes and candidate report (including warnings) |
| `edit ... --format toml` | Envelope v4: `operation` is `stage`, `diff`, `check`, or `apply`; original/candidate identities, reports, draft paths or publication outcomes |

Static reports carry `evidence.incomplete_reasons` as a deterministic, deduplicated
list, including when `status` is `fail`. Reasons distinguish `parser-error`,
`unresolved-license`, `unresolved-build-requirements`, and (for submit)
`unresolved-sources`; an empty list does not
expand the selected rule scope. Parser errors stop rule execution. Ordinary parser
warnings do not imply incomplete checks. Both generation and editing embed this
same versioned report, independently of their outer envelope version.

`gen --check` never writes or consults output conflicts; `--format` requires
`--check`, which conflicts with output/preview/overwrite flags. The outer `input` identifies the consumed TOML; the nested candidate report names
the intended SPEC destination, not an existing-file claim. `build_contract` is omitted in
explicit-stage mode. Profile/contract hashes identify embedded TOML, not an entire
environment or all validation code. With `--format toml`, input, rendering and
static failures yield one report and exit 1. Before a candidate exists,
`report` is omitted and `error` explains the failure; unreadable
input has no SHA-256. CLI argument and output-write failures can precede TOML.

Static evidence records `policy`, `source_uncertainty` (omitted when resolved), and
`not_checked` stages. RPK005 uses the same ordered Source resolver as source
hashing: known macros and conditions are evaluated without executing RPM macros;
unknown declarations remain explicit. Gen/edit retain authoring warnings for
missing SHA-256. A declared digest is never evidence that downloaded bytes match.

Reporting commands use an `error` object with `code` and `message`.
Unreadable/invalid-UTF-8 inputs report `valid = false`, `code = "input-read"`, and
no fabricated input hash. Generation uses report version 4; `source hash` uses
version 2; `source verify` uses version 1. Codes describe the failing boundary, not text matched from a message.
Machine stdout is one complete TOML document, not line-delimited records; parse it with a TOML reader (for example Python 3.11+ `tomllib`). Check reports already
include the baseline, so a separate original `check` is unnecessary for comparison.
Preparation (`scope: "edit-stage"`) returns absolute draft paths, not a validated
candidate. Edit report version 4 separates `success` (the requested operation completed) from
`valid` (all candidates passed the selected static checks). Each candidate also
reports `admissible`: whether the local edit may be saved. Unchecked staging/diff has omitted `valid` and `admissible` fields, not a fabricated
pass. A successful edit may
retain confirmed, unchanged authoring violations; it is not a passing package check.
Publication `outcomes` report `written`,
`unchanged`, or `skipped`, actual destination paths, and known resulting hashes.
On partial I/O failure, `success` is false and `written` lists confirmed writes;
remaining files are not claimed complete. A missing receipt does not prove no write
occurred. If stdout fails after publication, stderr names confirmed writes when
it is still writable; publication is not undone. Neither channel is guaranteed
when both are closed. Successful in-place application advances the stage baseline; a failed stage update
is reported separately from any confirmed SPEC write. Old immutable baselines
remain available for recovery.

For edit, `baseline_report` records the original checks; `introduced_static_blockers`
compares blocking rule facts, ignoring shifted positions (omitted when unresolved
checks prevent attribution). The editor shows its fixed selection before editing. Publication may retain
a known authoring violation only when that rule’s complete inputs are confirmed
unchanged. Changed invalid values, new blockers, incomplete checks and violations
without input evidence still block. `edit --check` tests this same local-edit
admission; standalone `check` and the submit policy remain strict.
For edit, `original_sha256` identifies the source; the nested input hash identifies
the candidate. Each file has `state = "pending-edit"` or `"candidate"`. Paths have no human presentation suffix.
Errors have `code`, `message`, and optional `path`/`selected_fields`; selection is
operation scope, not necessarily the offending field. Codes include
`unmappable-fields`, `source-changed`, `source-hash-failed`, `invalid-draft`,
`invalid-assignment`, `invalid-candidate`, `static-check-failed`, and the fallback
`operation-failed`. CLI argument errors can precede TOML. Diagnostics use lowercase
severity names and remain inside TOML rather than stderr.

Pin tool/parser identities and check `format_version`. Incompatible report changes
increment it; optional fields and unknown error codes must be tolerated. Saved
drafts and manifests have no cross-version compatibility guarantee in this preview.

The native RPM development gate limits each command to 120 seconds and kills its
process group on timeout. It is not a runtime fallback for Source operations.

### Machine reports

A safely constructed `edit --diff` remains inspectable when an explicitly requested
check fails; its exit status is 1. Without `--check`, a diff is unchecked, not a
claim that the package passes static rules. `--apply` always requires local-edit
admission. A construction or source-freshness failure produces no diff.

Publication errors retain their code/message and add `stage`, `reason`, and a
path when known; I/O failures also carry `io_kind`. On partial publication,
`written` is authoritative: a failure is not a rollback.

Source failures carry `stage`, `reason`, `retryable`, and `http_status` when
applicable. `retryable` concerns the download only, never the whole edit/apply.
Generation exposes best-effort failures in `source_hash_failures`; missing
SHA-256 remains a warning, not a publication policy.

TOML reports include the producer's name, version, full Git `revision`, and
`dirty` (tracked changes only, including staged changes). Static check reports
keep this under `evidence.tool`; other command envelopes use `tool`, including
input failures. Unknown revision/dirty fields are omitted, not a clean-tree claim.
Git is consulted only at build time and is optional; an archive never borrows an
ancestor repository's revision. Archive packagers can set
`RUYIPACK_SOURCE_REVISION` to a full object ID and optionally
`RUYIPACK_SOURCE_DIRTY=true|false` during the build. These are supplied provenance,
not independently verified claims or proof of a reproducible build.
