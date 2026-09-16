<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack

RuyiPack is a project for openRuyi RPM package workflows.

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

Generate a SPEC from an Autotools manifest:

```sh
cargo run -- gen ed --manifest examples/ed/ed.toml
```

The command validates the authoring fields, parses the generated SPEC, compares its
facts with the manifest and distribution defaults, and runs the selected static
checks before writing it. Macro expressions are compared without evaluating them.

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
keep the file, show a diff and return to the menu, write a copy, or overwrite it.
Keeping the file is highlighted initially and still requires confirmation. Cancelling, or encountering
a conflict without a usable terminal, returns an error without writing.

Menu copies use `ed.spec.new`, then `ed.spec.new.1`, and so on, without replacing
existing files. The selected copy path is printed to stderr. Prompts and messages
stay on stderr; candidate text and diffs go to stdout. Output failures return
status 1. A file remains written if reporting its path subsequently fails.

Source URLs may reference `%{name}`, `%{version}` and `%{url}`. The SPEC
preserves these expressions and URL filename fragments such as `#/name.tar.gz`.

Declare additional remote inputs as `[sources.1]`, `[sources.2]`, and so on, each
with `url` and `sha256`. Source numbers are preserved and printed in numeric order.
`sources.0` supplies the archive for the default unpacking step.

Edit the source-preserving author fields of a supported SPEC:

```sh
ruyipack edit ed.spec --view
ruyipack edit ed.spec --set package.version=1.22 --diff
ruyipack edit ed.spec --set package.version=1.22 --stdout
ruyipack edit ed.spec --set package.version=1.22 -o reviewed.spec
ruyipack edit ed.spec --set package.version=1.22 --force
ruyipack edit ed.spec --set package.version=1.22 --set spec.release=2
```

`--view` prints the supported author fields as TOML. `--set FIELD=VALUE` replaces
an existing string field; repeat it to change several fields. `--diff` prints a
source-to-candidate diff and `--stdout` prints the complete candidate; neither
writes files. `--force` overwrites the source without a confirmation menu.
`-o, --output FILE` selects another destination; missing targets are created and
identical targets are left unchanged. `--view` cannot accompany `--set` or any
publication option. `--diff` and `--stdout` cannot accompany another output mode;
`--force` can accompany `--output`.

For different existing content, the default terminal menu offers to keep it,
show a diff and return to the menu, write a `.new` copy, or overwrite the target.
Keep is selected by default. Without a terminal, a conflict fails without writing;
choose an explicit read-only preview or `--force`. Edits reject source changes
observed since the candidate was prepared, including when using `--force`.
On Unix, new edit outputs and copies preserve the source access bits, while
replacements preserve the target access bits; special mode bits are cleared.

Before previewing or publishing, RuyiPack renders the candidate with source-local changes,
reparses it, checks that the requested editable fields survived, and runs the
same selected static checks as `check`. Unsupported or ambiguous source forms
are rejected rather than rewritten approximately. RPM expressions stay unexpanded;
changing a version or URL does not fetch sources or validate a package build.
