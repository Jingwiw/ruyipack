<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# Command and manifest reference

[Quick start](../README.md#try-it) · [Design and policy sources](design.md)

This reference covers semantics beyond `ruyipack COMMAND --help`.

## Initialize

`init NAME` writes `NAME.toml`; `--dir` selects an existing output directory.
No build system is selected by default. `--build-system autotools|cmake|meson`
adds stage guidance; `--comments full` adds explanations and a commented
subpackage example without changing values. Autotools prefills `autoconf`,
`automake`, `libtool`, and `make`. CMake/Meson do not prefill a common tool set.
Declare actual dependencies and confirm macros in the target environment.

The scaffold saves the current year and configured Git author once. An unavailable
or unsuitable author leaves an empty contributor list and a warning. Review these
values and fill in package facts, sources, dependencies, stages, and files; `gen`
uses saved values, not the current identity or date.

`--specs-dir` selects the local package-name lookup directory; otherwise `init`
searches for the nearest `SPECS` above the output directory. Any existing entry,
including untracked entries, blocks that name even with `--force` or `--stdout`.
No lookup directory produces a warning; an invalid explicit directory is an error.
This is not a query of Git, an RPM repository, or upstream package availability.
TOML output uses the [shared output rules](#output-and-file-safety).

## Generate

`gen NAME` reads `NAME.toml`, or the file selected by `--manifest`. NAME is a
package selector, not a path. Generation validates the manifest, reparses and
compares candidate facts, then runs [static checks](#static-checks).
See [ed.toml](../examples/ed/ed.toml) for a complete manifest.

### Repository and Sources

Choose an entry in `[package.vcs]` only when the fact is known:

| Entry | Generated result |
| --- | --- |
| `git = "https://example.org/project.git"` | `VCS: git:...`; do not include the prefix in the input |
| `same-as-url = true` | Omit VCS; `package.url` already names the source repository |
| `no-public-repository = true` | Emit the profile's no-repository comment |

Omit the table or leave it empty while the repository status is unknown. Generation
warns (also in JSON `authoring_warnings`), emits no VCS assertion, and never turns
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

After filling the scaffold, `gen NAME` automatically attempts to download remote
Sources whose `sha256` is absent and fill the generated SPEC. Use `--offline` to
skip downloads entirely. Existing digests are retained, not downloaded or
verified; remove a digest from the TOML when you intend to recalculate it.
Completion uses built-in Rust HTTP/TLS and the package fields below, without
executing macros. Download failures warn per Source and leave its digest missing.
Connection setup is limited to 10 seconds; the complete transfer, including
redirects and reading the body, to 300 seconds.

Completion changes the same in-memory manifest used by the renderer: it never
writes the author TOML or an intermediate TOML. `--stdout` / `--diff` preview the
completed SPEC; `--check --format json` writes no files and includes successful
downloads in `source_hashes` (null with `--offline`, otherwise possibly empty).
Failure reasons are in `authoring_warnings`; a static pass does not mean every
digest was calculated. Preview and check modes also download unless `--offline`
is set. Repeating the command downloads again while hashes remain absent in the
author TOML; no hidden cache or writeback pins them. A calculated digest is not
source authentication.

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

`edit FILE.spec` in a terminal asks what to edit; it does not attempt a full conversion.
Scripts specify `--field`, `--set`, `--hash-source`, or `--all`. Use `--all --view` only when every
construct is supported; `inspect` is the read-only overview.

Use `--field FIELD` for an editor view, `--set FIELD=VALUE` for string assignments,
`--view` to read TOML, or `--schema` to read its JSON Schema. The editor is selected
from `--editor`, `$VISUAL`, `$EDITOR`, then `vim`; arguments are split without a
shell. GUI editors must wait, e.g. `--editor 'code --wait'`. A successful editor
exit validates and writes the candidate; editor output goes to stderr.

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
`source-hash` result. A mismatch refuses the operation; publication still checks
for subsequent changes. This binds the SPEC bytes, not external macros or URLs.

### Persistent drafts

```sh
ruyipack edit ed.spec other.spec --field package.version --prepare drafts
# Edit the TOML files in drafts, then:
ruyipack edit --from drafts --check --format json
ruyipack edit --from drafts --diff
ruyipack edit --from drafts
```

`--from` checks/applies without an editor; add `--editor COMMAND` to reopen it.
Drafts use source basenames and TOML 1.1. Keep `.state` (original bytes, source
identities, and schemas) with the editable files. Duplicate basenames need separate
directories. Prepare fresh drafts after source changes or applying a batch.
Editor work is retained after validation failure or when changes remain unapplied.

## Output and file safety

- `gen` defaults to `NAME.spec` beside its manifest; `init` defaults to `NAME.toml`.
  `edit` defaults to replacing its source. Explicit output paths are relative to
  the current directory; parent directories must exist.
- `--stdout` prints one candidate without consulting the target. `--diff` prints
  a unified diff without writing (missing target = new file); edits can preview a
  batch. Both return 0 on successful previews regardless of differences. Diff
  headers require UTF-8 paths without tabs/newlines.
- Different existing outputs require an explicit action or terminal confirmation.
  `--force` replaces; `--skip-existing` keeps existing files and creates missing
  ones. These apply to init/gen; edit's `--force` requires `--output`. Same-content
  writes leave files untouched. See `--help` for incompatible option combinations.
- Without a usable terminal, unresolved conflicts fail. The terminal menu offers
  keep (initial selection, still requires confirmation), diff, copy, or overwrite.
  Copies use `.new`, `.new.1`, etc. without replacing existing files. Cancellation
  fails without writing. Candidate/diff/JSON go to stdout; notices go to stderr.
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

## Static checks

`check`, generated candidates, and edit candidates share these checks:

| Rules | Scope |
| --- | --- |
| RPM010–RPM015 | Selected required main-package tags |
| RPK001 | SPDX expressions in package License tags and recognized top-level SPEC file-license comments; these are different licenses |
| RPK002 | Literal Name, Version, Release syntax; `Epoch: 0` is not rejected |
| RPK003 | Literal project URL syntax; existing HTTP/HTTPS accepted |
| RPK004 | Profile direct requirements for a single literal BuildSystem; currently Autotools has required tools |
| RPK005 | Warning for a missing or malformed adjacent Source SHA-256; unrelated edits can continue |

SPDX covers main/subpackages and conditional branches, not upstream license
correctness. IDs are case-insensitive, operators uppercase, deprecated IDs valid;
unknown IDs use bundled SPDX data. Missing file-license comments are not rejected
by this expression check; script comments are not declarations. Unresolved
license expressions leave checks incomplete even when a finding also proves failure.
Unchanged invalid literal metadata also fails candidate checks; macro values are
not evaluated or certified.

Missing Autotools tools fail unless unresolved conditions, rich dependencies, or
expressions could supply them, in which case evidence is incomplete. CMake/Meson
have no common required-tool set here. Unknown/context-dependent BuildSystem
selections are outside this contract, not inferred.

`pass` means only the selected rules passed; parser warnings may remain.
Standalone `check` warns about missing Source digests but does not fully validate
Source URLs/RemoteAsset associations or refresh archive digests. Static commands
do not download sources or expand native RPM macros. No command resolves dependencies, verifies patches, or builds packages.

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
ruyipack verify-sources --manifest package.toml --format json
ruyipack verify-sources package.spec -D 'archive_version 2.0'
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
failure exits 1. JSON includes input identity, definitions, original expressions,
separate `declared_sha256` and successful `download` evidence (SHA-256, URLs, byte
count). A changed input fails verification; retained results describe the recorded
input hash, not the new file. CLI errors exit 2 and may precede JSON.

## Computing or completing a Source digest

```sh
ruyipack source-hash package.spec --source 0 --format json
ruyipack edit package.spec --set package.version=2.0 --hash-source 0 --diff
ruyipack edit package.spec --hash-source 0 --prepare drafts
```

`source-hash` downloads one selected Source without writing. Human output is its
digest on stdout and copyable preview/apply commands on stderr, with an input hash
guard against stale edits. JSON failures contain an error, never a fabricated hash.

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

## JSON and inspection

`inspect --format json` returns main-preamble syntax, input SHA-256, parser identity,
and diagnostics, not macro definitions or section bodies. It does not evaluate
conditions. Recoverable parser errors do not fail inspection; use `check` for gates.
Human inspection normalizes tags and displays their conditional structure.

`inspect` node `data` spans refer to original UTF-8 bytes: zero-based, end-exclusive
offsets; one-based lines/byte columns. Verify the input digest before using them.
They are parser locations, not safe replacement ranges. Invalid diagnostic byte
bounds/order/UTF-8 boundaries and known unreliable macro coordinates yield `null`
spans without hiding messages. This does not establish general semantic accuracy.
`preamble` serialization is tied to the recorded parser revision.

| Report | Contract |
| --- | --- |
| `check --format json` | Report v2: input identity, parser, selected rules, `spec-static` evidence, findings and `parser_diagnostics` |
| `gen NAME --check --format json` | Envelope v2, `manifest-generation-static`: manifest/profile/selected build-contract hashes and candidate report (including warnings) |
| `edit ... --format json` | Envelope v2: `operation` is `check`, `prepare`, or `apply`; original/candidate identities, reports, draft paths or publication outcomes |

Static reports carry `evidence.incomplete_reasons` as a deterministic, deduplicated
list, including when `status` is `fail`. Reasons distinguish `parser-error`,
`unresolved-license`, and `unresolved-build-requirements`; an empty list does not
expand the selected rule scope. Parser errors stop rule execution. Ordinary parser
warnings do not imply incomplete checks. Both generation and editing embed this
same versioned report, independently of their outer envelope version.

`gen --check` never writes or consults output conflicts; `--format` requires
`--check`, which conflicts with output/preview/overwrite flags. Its input path is
the intended destination, not an existing-file claim. `build_contract` is null in
explicit-stage mode. Profile/contract hashes identify embedded TOML, not an entire
environment or all validation code. With `--format json`, input, rendering and
static failures yield one report and exit 1. Before a candidate exists,
`report_subject` and `report` are null and `error` explains the failure; unreadable
input has no SHA-256. CLI argument and output-write failures can precede JSON.

Missing Source SHA-256 is `RPK005`, a warning in all three commands, not a hard
failure or evidence of verified source content. Static detection covers adjacent
bare markers and literal HTTP(S) Source prefixes, not arbitrary macro expansion.

Reporting commands use an `error` object with `code` and `message`.
Unreadable/invalid-UTF-8 inputs report `valid: false`, `code: "input-read"`, and
no fabricated input hash. Generation uses report version 3; `source-hash` uses
version 2; `verify-sources` uses version 1. Codes describe the failing boundary, not text matched from a message.
Edit JSON is compact; use `jq` to select fields or format it. Check reports already
include the baseline, so a separate original `check` is unnecessary for comparison.
Preparation (`scope: "edit-draft"`) returns absolute draft paths, not a validated
candidate. `valid` describes the requested operation, not package quality.
Publication `outcomes` report `written`,
`unchanged`, or `skipped`, actual destination paths, and known resulting hashes.
On partial I/O failure, `valid` is false and `written` lists confirmed writes;
remaining files are not claimed complete. A missing receipt does not prove no write
occurred. If stdout fails after publication, stderr names confirmed writes when
it is still writable; publication is not undone. Neither channel is guaranteed
when both are closed. Old drafts remain stale after
application; use a new read or the receipt, not a forced stale-draft retry.

For edit, `baseline_report` records the original checks; `introduced_static_blockers`
compares blocking rule facts, ignoring shifted positions (`null` when unresolved
checks prevent attribution). Pre-existing errors are identified before opening the editor and on failure; they still block publication.
For edit, `original_sha256` identifies the source; the nested input hash identifies
the candidate (`report_subject`). Paths have no human presentation suffix.
Errors have `code`, `message`, and optional `path`/`selected_fields`; selection is
operation scope, not necessarily the offending field. Codes include
`unmappable-fields`, `source-changed`, `source-hash-failed`, `invalid-draft`,
`invalid-assignment`, `invalid-candidate`, `static-check-failed`, and the fallback
`operation-failed`. CLI argument errors can precede JSON. Diagnostics use lowercase
severity names and remain inside JSON rather than stderr.

Pin tool/parser identities and check `format_version`. Incompatible report changes
increment it; optional fields and unknown error codes must be tolerated. Saved
drafts and manifests have no cross-version compatibility guarantee in this preview.

The native RPM development gate limits each command to 120 seconds and kills its
process group on timeout. It is not a runtime fallback for Source operations.

### Machine reports

A safely constructed `edit --diff` remains inspectable when static checks fail;
its exit status is still 1. `--stdout` and publication still require those checks
to pass. A construction or source-freshness failure produces no diff.

Publication errors retain their code/message and add `stage`, `reason`, and a
path when known; I/O failures also carry `io_kind`. On partial publication,
`written` is authoritative: a failure is not a rollback.

Source failures carry `stage`, `reason`, `retryable`, and `http_status` when
applicable. `retryable` concerns the download only, never the whole edit/apply.
Generation exposes best-effort failures in `source_hash_failures`; missing
SHA-256 remains a warning, not a publication policy.
