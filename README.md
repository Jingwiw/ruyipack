<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack

Develop and build openRuyi RPM packages.

RuyiPack generates SPEC files from TOML and edits selected fields in existing SPEC files.
It preserves unrelated SPEC content. Experimental builds use Mock through Docker Compose.
It does not discover all build dependencies.

This source preview requires maintainer review. Commands and data formats can change.
Platform support is experimental.

## Install

Install Rust 1.98.1 and a C linker. From this repository, run:

```sh
cargo install --path . --locked
ruyipack --version
```

The first build needs network access for the toolchain and locked dependencies.
Add `$CARGO_HOME/bin` (normally `$HOME/.cargo/bin`) to `PATH`.
Keep `Cargo.lock` with source distributions.
See the [Ubuntu installation steps](docs/quickstart.zh-CN.md#安装).

## Try it

### Maintain an existing package

Start in an empty directory. Set `RECIPE_REPOSITORY` to your recipe Git repository URL.

```sh
ruyipack init . --clone "$RECIPE_REPOSITORY"
ruyipack open ed
```

Edit SPEC or Patch files in the editor. Alternatively, change metadata through a checked candidate:

```sh
ruyipack edit ed --set package.version=1.22.6 --hash --diff --apply
ruyipack check ed
ruyipack build ed
ruyipack shell ed
```

`--hash` replaces remote Source digests. It does not authenticate the source.
To compare existing digests without changing declarations, use `source verify ed`.
Static `check` does not build the package.

### Write a new recipe

A new local package does not need a recipe Git repository.
Start in an empty directory:

```sh
ruyipack init .
ruyipack new example --build-system cmake
ruyipack open example --authoring
ruyipack gen example --diff
ruyipack gen example --apply
ruyipack build example
```

Fill the unknown package facts in TOML before running `gen`.
Generation downloads Sources with missing digests unless you select `--offline`.
`gen --diff` leaves the recipe unchanged. `gen --apply` checks and writes the SPEC.

See [task plans](docs/tasks.md) to apply fixes, validate packages and collect a successful subset.

## Choose the operation

WORK is a named package development area. All named commands use its saved package binding.

| Task | Command |
| --- | --- |
| Initialize workspace configuration | `init [PATH]` |
| Create an area or import a recipe | `new WORK` |
| Edit recipe files directly | `open WORK` |
| Edit authoring TOML | `open WORK --authoring` |
| Change selected SPEC fields | `edit WORK` |
| Generate a SPEC candidate | `gen WORK` |
| Read SPEC facts | `inspect WORK` |
| Run offline rules or material checks | `check WORK` |
| Fetch materials, calculate or verify digests | `source fetch`, `source hash`, `source verify` |
| Build the recipe SPEC | `build WORK` |
| Enter the retained Mock chroot | `shell WORK` |
| Commit package changes to a clean Git repository | `commit WORK` |
| Remove build results, not package edits | `clean WORK` |
| Remove an entire development area | `delete WORK` |
| Export editor schemas | `schema manifest`, `schema edit` |

`edit` updates the WORK TOML; `--apply` also writes the checked SPEC.
`edit` and `gen` share the WORK TOML; cached outputs never select the input.
`build` reads the recipe SPEC. It does not run `gen` or apply TOML changes.
Use `--spec PATH` for an external SPEC, not a positional file path.

Repeat `build WORK` after changes. Earlier results are archived.
Use `clean WORK --history` to remove archived results.
Before deleting an area, inspect `delete WORK --dry-run`.
`delete` removes local package edits after confirmation; it does not delete repository branches or commits.
Review package changes with `commit WORK --dry-run`, then use `commit WORK`.
Commit uses the configured repository’s current branch; it does not push.

## Zsh completion

Generate completion from the installed binary; no workspace is required:

```sh
mkdir -p ~/.local/share/zsh/site-functions
ruyipack completions zsh > ~/.local/share/zsh/site-functions/_ruyipack
```

Add `fpath=(~/.local/share/zsh/site-functions $fpath)` before `compinit` in `~/.zshrc`.
Start a new shell. Regenerate completion after upgrading RuyiPack.
Completion includes commands, flags and declared values, but not WORK names.
For batch edit, put options before WORK names: `ruyipack edit --format toml ed`.
Static zsh completion stops after the variadic WORK argument.

## Documentation

- [中文快速入门](docs/quickstart.zh-CN.md): install, write, upgrade and debug a package.
- [Command and manifest reference](docs/reference.md): inputs, checks, file safety and machine reports.
- [Build guide](docs/build.md): environment setup, retained results and cleanup.
- [Design and development](docs/design.md): ownership, policy sources, tests and contributor gates.
- [Example manifest](examples/ed/ed.toml).

Use `--format toml` for machine reports and `--debug` for internal diagnostics on stderr.
Build reports link saved JSON receipts. Editor schemas use JSON Schema.
Static commands need neither native RPM nor Docker.

## License

[MulanPSL-2.0](LICENSE). See [LICENSES](LICENSES) for file-level license texts.

See [OBS remote builds](docs/remote-build.md) for `remote-build WORK` and multi-package plans.
