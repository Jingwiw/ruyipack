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
complete dependencies, download sources, or build packages. CLI, manifest, and
report formats may change; platform support is experimental.

## Install

From this checkout, with Rust 1.91.0, a C linker, and network access for the locked
registry and Git dependencies:

```sh
cargo install --path . --locked
ruyipack --version
```

Put `$CARGO_HOME/bin` (normally `$HOME/.cargo/bin`) on `PATH`. Keep `Cargo.lock`
with source distributions.

## Try it

Run the included example without changing the checkout:

```sh
work=$(mktemp -d)
cp examples/ed/ed.toml "$work/ed.toml"
ruyipack gen ed --manifest "$work/ed.toml"
ruyipack check "$work/ed.spec"
ruyipack inspect "$work/ed.spec"
ruyipack edit "$work/ed.spec" --set package.version=1.22.6 --diff
printf 'Example files: %s\n' "$work"
```

The last command previews a change; omit `--diff` to write it back. Version or
Source changes **do not refresh digests or verify patches**. Before submitting a
package, review its sources and changes, then build and test in the target
openRuyi environment. A static `pass` is not a successful package build.

## Commands

| Command | Use | Important boundary |
| --- | --- | --- |
| `init NAME` | Create a TOML scaffold | Fill in unknown package facts; local name checks are not upstream availability checks |
| `gen NAME` | Generate from `NAME.toml` | Supports manual subpackages and explicit stages; manifest is authoritative |
| `inspect FILE.spec` | Read tags and parser diagnostics | Syntax, not macro-expanded RPM values |
| `check FILE.spec` | Run selected static rules | Does not validate Source downloads or the complete package |
| `edit FILE.spec --field package.version` | Edit selected fields through TOML | SPEC is authoritative; ambiguous fields are refused, unselected bytes preserved |

Use `ruyipack COMMAND --help` for flags. Start a new package with `init`, complete
its manifest, then run `gen`. Existing complex SPECs are best edited with `--field`
or `--set`; full-view editing only supports a limited subset.

## Documentation

- [中文快速入门](docs/quickstart.zh-CN.md)
- [Command and manifest reference](docs/reference.md): stages, Sources, subpackages,
  drafts, output safety, static rules, and JSON contracts
- [Design and openRuyi context](docs/design.md): operation ownership, policy sources,
  module responsibilities, and regression contracts
- [Example manifest](examples/ed/ed.toml)

## Development

```sh
./scripts/check
./scripts/smoke-test "${CARGO_HOME:-$HOME/.cargo}/bin/ruyipack"
```

The gate runs formatting, locked tests, Clippy, REUSE, and cargo-deny. Install
`reuse` 6.2.0 and `cargo-deny` 0.20.2 separately. The smoke test exercises the
installed binary in temporary files.

In a trusted target environment with Python 3, `rpm`, `rpmspec`, and
`rpm-config-openruyi`, `./scripts/check-native-sources` checks native Source
numbering and records RPM/macro identities. It does not build packages.

Keep changes focused and include a regression test for behavior changes. For bug
reports, include `ruyipack --version`, OS/architecture, the exact command, minimal
input, and complete output. Remove credentials and private data before sharing.

## License

[MulanPSL-2.0](LICENSE). File-level declarations and license texts are recorded in
[LICENSES](LICENSES).
