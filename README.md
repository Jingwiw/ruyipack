<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack

RuyiPack inspects, checks, edits selected fields in, and generates SPEC files for openRuyi.

## Installation

Build from the source directory with Rust 1.91.0, a C linker, and network access
for the locked registry and Git dependencies:

```sh
cargo install --path . --locked
ruyipack --version
```

Cargo installs the executable in `$CARGO_HOME/bin` (normally `$HOME/.cargo/bin`).
Add that directory to `PATH`. Keep `Cargo.lock` with source distributions;
`--locked` uses the reviewed dependency versions.

To check a source checkout and test the installed executable:

```sh
./scripts/check
./scripts/smoke-test "$HOME/.cargo/bin/ruyipack"
```

`scripts/check` runs formatting, locked tests, Clippy, `reuse lint`, and
`cargo deny --locked check`; it requires the `reuse` and `cargo-deny` tools on `PATH`.
The smoke test uses temporary files and does not modify packages in the checkout.
The commands below perform static analysis without executing RPM macros or builds.

## Inspect


Inspect an existing SPEC without changing it:

```sh
ruyipack inspect ed.spec
ruyipack inspect ed.spec --format json
```

Human output shows normalized main-package tags and their conditional structure.
JSON includes the same parser tree, the input path and SHA-256, the parser version
and revision, and all parser diagnostics. Diagnostics use the same lowercase
severity names as `check --format json` and stay in JSON rather than stderr.
Parser warnings and recoverable errors do not change inspection's success status;
use `check` to apply the selected static rules.

JSON `value` fields are parser syntax, not evaluated RPM values. Node `data` spans
refer to the original UTF-8 input: byte offsets are zero-based and end-exclusive;
line and byte-column numbers are one-based. Verify the input digest before using
these offsets to read source text. Spans are parser locations, not guaranteed
value-only or safe replacement ranges. The view omits macro definitions and section
bodies and does not evaluate conditions. The preamble tree uses the recorded
`rpm-spec` revision's serialization format.

## Edit

Select existing fields to edit through TOML:

```sh
ruyipack edit ed.spec --field package.version
ruyipack edit ed.spec --set package.version=1.22.6 --diff
ruyipack edit ed.spec --set package.version=1.22.6
ruyipack edit ed.spec --field package.version --view
```

The editor command is selected from `--editor`, `$VISUAL`, `$EDITOR`, then `vim`.
For VS Code, use `--editor 'code --wait'`; close the edited tabs to return to the
command. After the editor exits successfully, checked edits are written to the
source SPEC. Editor output stays on stderr.
Commands are split into arguments without running a shell.

`--set FIELD=VALUE` replaces an existing string field; repeat the option for more
fields. Values are literal strings, including `=` characters after the first one.
Use the editor for arrays. `--field` selects existing supported fields or tables
and can be repeated. Unselected fields retain their original values.

Before publication, edit checks the TOML shape, renders source-local replacements,
parses the candidate, compares the edited fields, and runs the same static checks
as `check`. Text outside the selected replacements is preserved. Macro expressions
remain expressions: this does not download archives, verify patch applicability,
evaluate macros, or build packages.

The editable subset includes main-package metadata, numbered remote Sources with
adjacent SHA-256 markers, declarative BuildSystem, BuildRequires, descriptions,
simple file lists, header metadata, comments, and changelog text. Without `--field`,
editing opens a full view, which requires a mapping for the whole source. This is
limited to simple SPECs; VCS tags and build scripts require a selected-field view. `--field` and `--set` map only the selected fields,
so unrelated constructs such as VCS tags or build scripts remain untouched.
Ambiguous selected fields and parser errors stop the operation. Deleting keys or
adding unmapped groups is rejected; supported existing lists can change.

Use `--diff` or `--stdout` for a preview, or `-o FILE` for a separate destination.
Replacing a different existing output file requires confirmation; `--force` with
`--output` allows replacement without prompting. `--stdout` and
`--output` accept one SPEC; `--diff` can preview a batch. Source changes detected
after export or while waiting for the editor or menu stop publication, even with
`--force`. Editor work is retained when validation fails or edits remain unapplied.

Prepare ordinary files for longer or batch editing:

```sh
ruyipack edit ed.spec other.spec --field package.version --prepare drafts
code drafts
ruyipack edit --from drafts --check
ruyipack edit --from drafts --check --format json
ruyipack edit --from drafts --diff
ruyipack edit --from drafts
```

`--from` checks and applies the saved drafts without opening an editor. To edit
those drafts again, add `--editor COMMAND`.

Each draft is named after its source file. The `.state` directory keeps original
bytes, source identities, and JSON Schemas separate from editable fields. Keep it
with the drafts. The directory is local to these source paths; prepare new drafts
after the sources change or after applying a batch. Duplicate source basenames
require separate draft directories. `--schema` prints a schema for one displayed
view. Generated drafts declare TOML 1.1 and a local schema for compatible editors.

Batch candidates are all checked before publication. Writes are atomic per file,
not across the batch; a later I/O failure reports files already written. Inspect
those files before retrying. Drafts do not update SPEC files in the background;
write-back happens only after the command validates them.

## Initialize

Create a manifest to fill in before generating a SPEC:

```sh
ruyipack init example
ruyipack init example --build-system autotools
ruyipack init example --comments full --stdout
ruyipack init example --dir packaging --specs-dir /path/to/openRuyi/SPECS
```

`init NAME` creates `NAME.toml` in the current directory. `--dir` selects an
existing output directory; it does not create directories. The template contains
current authoring fields and optional explicit build stages. No build system is
selected by default. `--build-system autotools` selects Autotools, prefills its
required tools from the shared contract, and shows its default actions and optional
stage overrides. Add archive tools and package-specific dependencies as needed.
`--comments full` adds guidance without changing the field values.

The current year and configured Git author are filled in once. Review these
values, then fill the remaining package information, source digest, build commands,
dependencies, and installed files. If Git identity is unavailable or unsuitable,
the contributor list stays empty and a warning asks you to fill it. Generation
uses the saved values, not the current Git identity or date.

Before generating the template, `init` checks for `NAME` in `--specs-dir`, or in
the nearest `SPECS` directory found above the output directory. This reads working
directory entries, including untracked packages, not a Git index or published RPM
repository. Any existing entry blocks initialization, including with `--force`
or `--stdout`. Local absence does not prove absence upstream; no network request
is made. Without a discovered `SPECS` directory, initialization continues with a
warning that name availability was not checked. An invalid explicit directory is
an error.

Existing TOML files use the same conflict handling as `gen`: `--stdout`, `--diff`,
`--force`, `--skip-existing`, and the terminal menu. Fill the template and run
`ruyipack gen NAME` from its directory; incomplete input fails without creating a
SPEC.

## Generate

Generate a SPEC from an Autotools manifest:

```sh
ruyipack gen ed --manifest examples/ed/ed.toml
```

The command validates the authoring fields, parses the generated SPEC, compares its
facts with the manifest and distribution defaults, and runs the selected static
checks before writing it. Macro expressions are compared without evaluating them.

Select one repository declaration in `[package.vcs]`:

```toml
[package.vcs]
git = "https://example.org/project.git"
# same-as-url = true
# no-public-repository = true
```

`git` takes an HTTPS checkout address without the `git:` prefix and generates a
`VCS: git:...` tag. Use `same-as-url = true` instead when `package.url` already
points to the source repository; the VCS tag is then omitted. Use
`no-public-repository = true` instead when no public repository is available;
this generates the distribution's no-repository comment. Empty or conflicting
declarations are errors. Addresses are checked locally, not contacted.

Pass options to the default Autotools stages:

```toml
[build.stages.conf]
options = ["--enable-largefile", "--enable-nls"]

[build.stages.build]
options = ["CC_FOR_BUILD=gcc"]
```

Stages are `prep`, `conf`, `build`, `install`, and `check`. Each array entry is one
single-line RPM option string, emitted as `BuildOption(stage):  ...` in array order.
Stage tables are emitted in build order. RPM macros and shell quoting are preserved;
entries are not automatically quoted as individual shell arguments. Empty arrays
add no options. The distribution's default actions and options remain in effect.

Add commands before or after a default stage with `prepend` and `append`:

```toml
[build.stages.conf]
prepend = 'autoreconf -fiv'
options = ["--enable-nls"]

[build.stages.install]
append = '''
rm -f %{buildroot}%{_infodir}/dir
'''
```

These emit `%stage -p` and `%stage -a`; the default action stays between them.
Scripts use LF line endings. Indentation, comments, and RPM macros are preserved;
empty strings add no script. Generation checks static section boundaries and exact
text, not macro expansion, shell correctness, or build success.

Replace a default action with `replace`:

```toml
[build.stages.conf]
replace = '# Upstream has no configuration step.'
```

This emits `%conf` without a flag. Omit `replace` to keep the default action;
`replace = ""` emits an empty section to skip it. `prepend` and `append` still run
before and after the replacement. Non-empty `options` cannot accompany `replace`
in the same stage; put arguments in the replacement script instead. When skipping
tests, keep the reason in a script comment so it appears in the SPEC.

To write all stage commands explicitly, omit `build.system`:

```toml
[build.stages.prep]
replace = '%autosetup -p1'

[build.stages.build]
replace = '%make_build'

[build.stages.install]
replace = '%make_install'
```

Without a build system, only the stages you supply are emitted; there are no
default unpacking, configuration, build, install, or test actions. `replace` sets
the main script, and `prepend` and `append` add scripts before and after it.
`options` requires `build.system`; put arguments directly in explicit scripts.
Omitting `[build]` emits no build stages. Declare the tools your commands need
in `[build-requires]`; no build-system requirements are added or enforced.

`gen NAME` reads `./NAME.toml` by default; `--manifest` selects another input file.
The SPEC is written beside the manifest unless `-o, --output FILE` selects another
path. Relative output paths are resolved from the current directory. The parent
directory must already exist. Identical content is left unchanged.

On Unix, replacements preserve the target's read/write/execute permission bits
(`0777`) and clear set-user-ID, set-group-ID, and sticky bits. New files use normal
creation permissions subject to the caller's umask. Ownership, ACLs, and extended
attributes are not copied from the replaced file.

```sh
ruyipack gen ed --stdout
ruyipack gen ed --diff
ruyipack gen ed -o ed.spec.new
ruyipack gen ed --force
ruyipack gen ed --skip-existing
```

`--stdout` prints the complete candidate without reading or writing the target.
`--diff` prints a unified diff without writing files; a missing target is shown as
a new file. Use `--diff --output FILE` to compare against a different target.
Successful previews exit with status 0, whether or not a diff contains changes.
Diff headers require UTF-8 paths without tabs or line breaks.

`-f, --force` allows replacement of different content. `--skip-existing` keeps an
existing file and exits successfully without prompting; a missing target is
created normally. Both options can accompany `--output`, but cannot be combined
with each other or with preview options. `--stdout` cannot accompany `--diff` or
`--output`. All modes validate the manifest and generated SPEC.

When content differs and no action is specified, the command shows the conflicting
file and available options. If stdin and stderr are terminals, a menu offers to
keep the file, show a diff, write a copy, or overwrite it. Keeping the file is
highlighted initially and still requires confirmation. Cancelling, or encountering
a conflict without a usable terminal, returns an error without writing.

Menu copies use `ed.spec.new`, then `ed.spec.new.1`, and so on, without replacing
existing files. The selected copy path is printed to stderr. Prompts and messages
stay on stderr; candidate text and diffs go to stdout. Output failures return
status 1. A file remains written if reporting its path subsequently fails.

Source URLs may reference `%{name}`, `%{version}` and `%{url}`. The SPEC
preserves these expressions and URL filename fragments such as `#/name.tar.gz`.
Generation and editing share URL and SHA-256 validation. New manifests require
HTTPS; editing also accepts existing HTTP sources. Referenced package fields must
be available as unambiguous static literals. Literal percent escapes use `%%`
(for example, `a%%20b.tar.gz`); unhandled macro tokens are reported as unsupported.
SHA-256 values contain 64 hexadecimal digits, with their original case preserved.

Declare additional remote inputs as `[sources.1]`, `[sources.2]`, and so on, each
with `url` and `sha256`. Source numbers are preserved and printed in numeric order.
`sources.0` identifies the primary source. The Autotools default unpacking step
uses it as the source archive.

## Check

```sh
ruyipack check ed.spec
ruyipack check ed.spec --format json
```

`check`, `gen`, and `edit` share the selected SPEC checks, including SPDX
expressions in package `License` tags (`RPK001`). Literal expressions are checked
in main packages, subpackages, and conditional branches. Unresolved values prevent a successful check; the status is incomplete unless
another finding already proves a failure. No macros are executed. This
checks the declaration, not whether it matches the upstream source license.
License IDs are case-insensitive; operators use uppercase. Deprecated IDs remain
valid. Unknown names are checked against the SPDX data bundled with the tool.

Literal Name, Version, and Release values share lexical checks (`RPK002`).
Literal project URLs use the same URL parser as Source URLs (`RPK003`): existing
HTTP and HTTPS URLs are accepted; new manifests require HTTPS. An unchanged
invalid literal also fails the candidate check. Values containing RPM expressions
are not evaluated or certified by these lexical checks. `Epoch: 0` is not rejected.

A single literal `BuildSystem: autotools` activates the profile's direct build
requirements (`RPK004`). Missing tools are errors. If conditions, rich dependencies,
or unevaluated declarations could supply them, the result is incomplete rather
than a claim that they are absent. Other build systems and context-dependent
BuildSystem selections are outside this contract check. This is a declaration
check, not dependency resolution or proof that a package builds.
