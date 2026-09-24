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

Choose exactly one entry in `[package.vcs]`:

| Entry | Generated result |
| --- | --- |
| `git = "https://example.org/project.git"` | `VCS: git:...`; do not include the prefix in the input |
| `same-as-url = true` | Omit VCS; `package.url` already names the source repository |
| `no-public-repository = true` | Emit the profile's no-repository comment |

Addresses are checked locally, not contacted. Empty/conflicting choices fail.

Each `[sources.N]` has `url` and optional `sha256`. Generation writes explicit
Source numbers in numeric order. `sources.0` is the primary archive used by the
Autotools default unpacking step. A missing digest produces a bare `#!RemoteAsset`
and warning; an empty digest fails. Digests must be 64 hexadecimal digits; case
is preserved.

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
and `entries`. Empty/omitted subpackage files emit an empty `%files` for a
metapackage; the main package needs at least one file.

Per-subpackage license/URL/architecture, conditional declarations, and
macro-generated families are unsupported. File ownership still needs build-time
verification.

## Edit

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

Unnumbered `Source:` uses its effective RPM number: after `Source3:`, it is 4,
not 0. Conditions, includes, or unsupported expressions can make implicit numbers
uncertain; affected Source edits fail rather than guess. Unrelated fields may
still be edited. Recognizable invalid URL/digest values can be viewed and repaired
if their ranges are unambiguous; selected replacements must validate. Unselected
digests are preserved, not certified. A bare RemoteAsset has no digest field to
select or add.

Version or Source URL changes produce `review_triggers` and `review_required`
(source/digests, patches, native build). They do not turn a static pass into a
failure. Empty lists mean no triggering change, not that external checks ran.

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
Standalone `check` does not check Source URLs/RemoteAsset associations or refresh
archive digests. No command here downloads sources, checks patches, expands native
RPM macros, resolves dependencies, audits all credentials, or builds packages.

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
| `gen NAME --check --format json` | Envelope v1, `manifest-generation-static`: manifest/profile/selected build-contract hashes, warnings, candidate report |
| `edit ... --check --format json` | Envelope v2, `selected-edit-static`: per-file original hash, candidate report, review lists and structured errors |

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
environment or all validation code. Static failure yields JSON and exit 1;
input/rendering failures may precede any JSON and use stderr.

For edit, `original_sha256` identifies the source; the nested input hash identifies
the candidate (`report_subject`). Paths have no human presentation suffix.
Errors have `code`, `message`, and optional `path`/`selected_fields`; selection is
operation scope, not necessarily the offending field. Codes include
`unmappable-fields`, `source-changed`, `invalid-draft`,
`invalid-assignment`, `invalid-candidate`, `static-check-failed`, and the fallback
`operation-failed`. CLI argument errors can precede JSON. Diagnostics use lowercase
severity names and remain inside JSON rather than stderr.

Pin tool/parser identities and check `format_version`. Incompatible report changes
increment it; optional fields and unknown error codes must be tolerated. Saved
drafts and manifests have no cross-version compatibility guarantee in this preview.
