<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack

Write, check, and edit openRuyi SPEC files without rewriting unrelated content.

This is an early **source preview** for maintainer-reviewed work. It generates
SPECs from TOML and edits supported fields in existing SPECs; it does not discover
complete dependencies. The experimental `build` command delegates prepared SPECs and
materials to Mock through Docker Compose. Generation downloads Sources with missing digests
unless `--offline` is set. CLI, manifest, and
report formats may change; platform support is experimental.

## Install

From this checkout, with Rust 1.98.1, a C linker, and network access for the locked
registry and Git dependencies:

```sh
cargo install --path . --locked
ruyipack --version
```

Put `$CARGO_HOME/bin` (normally `$HOME/.cargo/bin`) on `PATH`. Keep `Cargo.lock`
with source distributions. For Ubuntu prerequisites and Rust setup, see the
[installation walkthrough](docs/quickstart.zh-CN.md#安装).

## Try it

Start in an empty workspace with a committed recipe repository:

```sh
ruyipack init . --clone "$RECIPE_REPOSITORY"
ruyipack edit ed                            # edit persistent TOML; no SPEC write
ruyipack edit ed --set package.version=1.22.6 --diff
ruyipack gen ed                             # resolved TOML + cached SPEC
ruyipack gen ed --spec=.                    # SPEC beside the TOML
ruyipack edit ed --apply                    # checked, explicit checkout publication
```

`--diff` saves and displays a diff of an actual cached candidate SPEC. `--check`
runs local-edit admission after editing; `--apply` always checks. Source changes
require review: use `source verify` before deliberately refreshing declarations
with `edit --hash`. Static success is not a native package build.

## Commands

| Command | Use | Important boundary |
| --- | --- | --- |
| `init [PATH]` | Initialize `.ruyiconfig` in an empty directory | Offline unless `--clone`; repeated initialization never overwrites configuration |
| `build WORK` | Build the bound checkout using Mock and Docker’s current context | Experimental; prepared SPEC paths are also supported; see [build setup](docs/build.md) |
| `shell WORK` | Enter its retained Mock chroot at the RPM build directory | Requires a build without `--rm`; debugging changes are not a verified rebuild |
| `new WORK [--pkgname PKG]` | Create a development checkout and TOML scaffold | Requires committed main; implicit PKG must match SPECS/PKG/PKG.spec |
| `gen WORK` | Generate from WORK's selected authoring TOML or edit stage | Cached outputs by default; `--spec=auto` publishes checkout |
| `inspect WORK` | Read facts; `--editable --field FIELD` prints TOML | No macro execution; first read creates a binding, not checkout |
| `check WORK` | Selected offline rules; optional material inventory | Static pass is not a package build |
| `source hash WORK` / `source verify WORK` | Calculate a digest / compare all declared digests | Network, read-only; never replace declarations |
| `edit WORK` | Stage edits through TOML, inline `--field`/`--menu`, or `--set` | Only `--apply` publishes; unselected SPEC bytes preserved |
| `clean WORK` | Remove build results and resources | Keeps checkout, manifest and development branch |
| `schema manifest` / `schema edit WORK --field FIELD` | Full authoring / narrow editing schema | Structural assistance, not native validation |

Source downloads use Rust HTTP/TLS, without curl, RPM, or a container.
Native RPM runs inside the embedded or explicitly overridden build environment, or the separate
developer cross-check below; static commands still require neither RPM nor Docker.

WORK is the same object across commands. First use defaults PKG to WORK and
requires committed `SPECS/PKG/PKG.spec`; use `--pkgname PKG` to bind a different or
new package explicitly. Read-only commands create only `work/WORK/.config.toml`
and read committed main (its revision appears in machine reports). Writes create
a sparse checkout; existing checkouts retain their current branch and edits.

```sh
ruyipack new ed-test --pkgname ed
# Fill work/ed-test/ed.toml.
ruyipack gen ed-test --diff
ruyipack gen ed-test --spec=auto --force
ruyipack check ed-test
```

Use `--spec PATH` for independent SPEC files, not a guessed positional path.
`edit --spec PATH` is repeatable for batches. `gen` only accepts WORK, selected by
its saved binding and current input (`--input authoring|edit` selects explicitly). Completion preserves author input; `--offline` disables downloads. Build consumes SPEC, never
implicitly runs gen. Complex recipes use selected editing, not full conversion.

## Documentation

- [中文快速入门](docs/quickstart.zh-CN.md)
- [Command and manifest reference](docs/reference.md): stages, Sources, subpackages,
  drafts, output safety, static rules, and TOML report contracts
- [Design and openRuyi context](docs/design.md): operation ownership, policy sources,
  module responsibilities, and regression contracts
- [Example manifest](examples/ed/ed.toml)

Before queuing a build, `ruyipack check --spec package.spec --materials --source-dir SOURCES`
checks staged Source/Patch files offline and reports their SHA-256 inventory.
See [local material checks](docs/reference.md#local-build-materials) for scope and limitations.

## Development

```sh
git config --local core.hooksPath .githooks
./scripts/lint
./scripts/check
./scripts/smoke-test "${CARGO_HOME:-$HOME/.cargo}/bin/ruyipack"
```

`scripts/lint` is the shared pre-commit and CI Rust gate: formatting and Clippy
across the workspace, all targets and all features, with warnings denied. The
hook checks a fixed snapshot of the staged files, so partial staging and unrelated
working changes are allowed. It never stashes or rewrites your files or index;
if staged contents change during checks, it refuses the commit and asks you to
retry. Staged builds use `target/pre-commit` unless `CARGO_TARGET_DIR` is set.
The local Git configuration above enables it for this clone (review existing hooks
first).

`Cargo.toml` owns the lint policy: default Clippy rules plus selected checks for
unnecessary ownership/cloning, avoidable string allocations, and lossy integer
conversions. Broad `pedantic` is an advisory review, not a zero-warning target:

```sh
cargo clippy --workspace --all-targets --all-features --locked -- -W clippy::pedantic
```

Fix the cause before suppressing a lint; any necessary exception should be local
and explain the invariant. Keep the pinned toolchain for the commit gate.
`scripts/check` additionally runs locked tests, schema/gen comparisons, REUSE, and cargo-deny.
Install Python 3.11+ with `reuse==6.2.0` and `jsonschema==4.26.0`, plus
`cargo-deny` 0.20.2 separately. The smoke test exercises the
installed binary in temporary files.

In a trusted target environment with Python 3, `rpm`, `rpmspec`, and
`rpm-config-openruyi`, `./scripts/check-native-sources` checks native Source
numbering and records RPM/macro identities. Pass prepared SPEC paths and
`--require-package NAME` for additional parses, or `--ruyipack /path/to/ruyipack`
to generate and build the small local semantic fixture. CI runs this gate in the
pinned environment in `tests/native/Dockerfile`, offline and without root.
It checks Patch/stage order, subpackage ownership, file lists and flags—not
ecosystem build coverage or OBS release/changelog services. Parser diagnostics,
including those with exit status 0, prevent acceptance. Removed pinned packages
require an explicit environment update and revalidation, not a skipped gate.

Keep changes focused and include a regression test for behavior changes. For bug
reports, include `ruyipack --version`, OS/architecture, the exact command, minimal
input, and complete output. Remove credentials and private data before sharing.

## License

[MulanPSL-2.0](LICENSE). File-level declarations and license texts are recorded in
[LICENSES](LICENSES).

Machine reports use `--format toml`, including build outcomes and clean results.
A build outcome links the complete saved `receipt.json`; it does not serialize
that receipt into TOML. JSON Schema remains JSON; persistent backend/engine/host
receipt protocols are unchanged and their TOML migration is still pending.
