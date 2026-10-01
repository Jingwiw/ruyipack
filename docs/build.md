<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-License-Identifier: MulanPSL-2.0
-->

# Native development builds

`build WORK` resolves a saved package binding through the nearest workspace root,
then builds the SPEC in its development checkout. A missing area is created only
after a unique committed recipe SPEC is found. It does not fetch missing materials
or change the recipe. Static `check` remains independent of Docker, Mock, and
native RPM.

## One command

Install the standard Docker CLI and Compose plugin with access to a Linux daemon.
Initialize a workspace and prepare its configured recipe Git repository first:

```sh
cd /path/to/workspace
ruyipack build busybox
# A separate development area for the same package:
ruyipack build busybox-test --pkgname busybox
```

The defaults are `recipes = "openruyi"`, `work = "work"` and `specs = "SPECS"` in
`.ruyiconfig/config.toml`. A plain WORK name always means a workspace development
area, even if a file with that name exists in the current directory. Discovery
works from nested directories. New areas use PKG = WORK unless `--pkgname PKG` supplies
the initial binding; existing areas reuse their saved binding and cannot be
rebound. The input SPEC is `SPECS/PKG/PKG.spec` under the configured checkout,
not a guessed RPM Name or a file from the current directory. Without an implicit
match in committed main, use an explicit `--pkgname` binding.

Results live in `work/WORK/build/` (under the configured work directory), beside
`work/WORK/checkout/`, and keep the existing `input/`, `host/`, `engine/` and
`receipt.json` structure. Machine stdout is a TOML outcome with `operation`,
`success`, the producer identity, observed failures, and `receipt` pointing to the
full saved JSON receipt; persistent receipt protocols are not migrated yet. Prepared materials are copied recursively from
`checkout/SPECS/PKG/` (using configured `specs`), including local patches and
already present source files. No remote download, automatic source-directory
flattening, or Source digest acceptance is implied. The workspace's
`.ruyiconfig/build/` assets are copied into the result's `.config/` and their actual
hashes are recorded; user edits and original permissions are preserved.

Named builds do not accept `--source-dir` or `--dir`: the binding has one checkout
package directory and one retained build result. An existing result is never
overwritten; return with `shell WORK`, or explicitly clean it before another
attempt. `--timeout 1800` bounds backend execution; bounded termination, artifact
retrieval and cleanup may run afterward. Use `--format toml` for a typed outcome linking the complete saved receipt
on stdout; progress goes to stderr.

For an explicitly prepared SPEC outside the managed name route, use its path:

```sh
ruyipack build --spec /path/to/package.spec \
  --source-dir /path/to/SOURCES \
  --dir ./build
```

This advanced mode still requires `--source-dir`, rejects `--pkgname`, and stores one
result in `DIR/SPEC_STEM` (`DIR` defaults to `build`; its parent must exist).
A relative `--spec ./package.spec` selects the same mode. It does
not create or borrow a WORK binding, and without `--config` uses the embedded
openRuyi environment. Use another `--dir` or clean that result for another attempt.

By default the worker is **stopped and retained**, including its Mock chroot.
The receipt includes commands to restart or remove it. Add `--rm` to remove the
build's containers, networks and temporary volumes after evidence retrieval.
`--rm` is an explicit disposal policy, including failed builds, timeouts and
failed retrieval. Retrieval errors are still reported; unavailable evidence may
be lost. Neither mode deletes host results or shared images.

To remove a build's retained resources **and** its host result directory:

```sh
ruyipack clean busybox          # show scope and confirm
ruyipack clean busybox --force  # skip confirmation
```

Noninteractive cleanup requires `--force`. This flag does not bypass directory
or daemon identity checks. Cleanup uses the recorded project labels, not commands
from the receipt or a potentially changed Compose file. Results are local trusted
state; do not clean a build receipt supplied by an untrusted party. A Docker
connection is required even when no container remains. Shared images are retained.
Save `--format toml` stdout separately if you need a record of successful cleanup.

Inputs are copied, not mounted from the client filesystem. Thus the backend does
not assume a remote daemon can see local paths. It invokes standard Docker and
Compose only; the daemon's VM provider is not part of the interface. The command
uses Docker's existing connection configuration unless `--context NAME` is given
for an explicit override. The user's current context is never changed. Personal
context names and credentials belong to Docker configuration, not the project.

Optional preflight: `ruyipack check --spec package.spec --materials --source-dir SOURCES`.
This checks declared material presence and digests without downloading. Building
is a separate native operation, not proof that every static or source-integrity
rule passed.

## Return to the build directory

```sh
ruyipack shell busybox
# Explicit prepared-SPEC result: receipt package key, not WORK:
ruyipack shell package --dir ./build
```

Without `--dir`, `shell WORK` reuses the saved binding and the fixed WORK/build
receipt under the workspace root; it never creates a missing area. With explicit
`--dir`, its positional argument is the receipt's recipe key and the result is
`DIR/RECIPE`. This is a separate advanced mode, not fallback for a missing WORK.
Named build and shell hold the cooperative area lock throughout the operation;
shell also locks its receipt to serialize sessions.

This restarts the retained worker and enters **Mock's chroot**, at RPM's native
`%{_builddir}`. It does not merely open a shell in the outer container, create a
new environment, or rebuild. It requires a build without `--rm` that reached
chroot initialization. A removed worker or missing build directory is an error.
The recorded daemon and worker ownership must match; a running worker is refused.
A terminal gets an interactive TTY; piped input works without one, but Mock still
starts an interactive shell and may print prompts or job-control warnings. Shell
streams are not a JSON command-execution protocol.

On ordinary shell exit, the worker is stopped again, not deleted. The original
build receipt remains historical evidence; each session writes a separate
`host/shell-*.json` with its status and lifecycle commands. Interactive streams
are attached directly, not recorded as a transcript. Changes in this debugging
session are **not** exported patches or a verified clean rebuild. Shell interaction
has no time limit; `--timeout` bounds each probe and stop. Abrupt host termination
can leave the worker running; shell does not promise build's cancellation recovery.

## Advanced configuration

Named builds select ROOT/.ruyiconfig/build/compose.yaml by default and snapshot
that directory into the result's `.config/`. The original workspace assets are
never overwritten by embedded defaults or normalized in place. Keep relative
asset references within that directory so the copied context is self-contained.
Advanced explicit-SPEC builds without `--config` instead materialize the four
embedded `environments/openruyi/` files. In both default modes only this small
`.config/` directory is the Docker build context; recipe materials and logs are
not sent as image inputs. The receipt records the copied file hashes.

`--config /path/to/compose.yaml` overrides either default with a standard Compose
file. Its parent is the project/build context. The explicit file is not rewritten;
its hash is recorded, but arbitrary referenced files are **not** snapshotted.
Keep a custom context narrow and version-controlled.

Before image creation, `build` checks Compose availability, daemon identity/Linux
OS, and Compose configuration validity. Actual worker execution checks Python,
Mock, RPM and the target configuration. These checks do not predict repository
availability, supported emulation, every SPEC macro, or eventual build success;
those require their real native stages.

## The three boundaries

- **Engine (`src/build/mock.rs`, `mock.py`)** stages an executable invocation,
  creates an SRPM, runs Mock's full rebuild, records RPM facts, and verifies the
  returned receipt and artifact hashes. It does not call Docker or know a context.
- **Backend (`src/build/compose.rs`, `compose/clean.rs`)** builds/starts the `worker` service, copies
  input, executes argv, retrieves output, and cleans up. It has no RPM semantics.
- **Environment (`environments/openruyi`)** supplies the Dockerfile, standard
  Compose definition, target repositories, macros and privileges. These are not
  embedded in backend logic or substituted by the authoring profile.

A new engine implements staging, result verification and its shell argv; a new backend
implements validation, execution and shell attachment, and supplies its name. Select it at the CLI composition point. These are
private Rust boundaries, not a plugin ABI or a promise that every combination
works. An environment must provide the selected engine's executable requirements.

The Compose backend expects one service named `worker`, a long-lived default
command, readable `/input` and writable `/output`. It transfers files with Docker
copy, so no daemon socket or host source mounts are required. Keep the environment
build context narrow; it is trusted executable configuration.

The Mock engine requires Python 3.11+, RPM, a privilege-aware `mock` command,
`MOCK_CONFIG` naming a Mock configuration, and `BUILD_TARGET` naming target JSON.
The supplied image uses a normal `builder` user and a sudo wrapper restricted to
Mock's entry point. This is a convenience boundary, not a sandbox against hostile
Mock configuration or SPEC macros.

## Evidence and failure

The output contains:

- `.config/`: the selected workspace asset snapshot or embedded defaults (absent with explicit `--config`);
- `input/`: the exact staged recipe, materials and engine program;
- `receipt.json`: input SHA-256 identities, tool identity, actual argv/exit status,
  timeouts/cancellation, Docker runtime identity and transport/cleanup results;
- `host/`: raw Docker/Compose stdout and stderr, daemon and container inspect JSON;
- `engine/receipt.json`: Mock stages, target/config identity, installed packages,
  native RPM identities and artifact sizes/SHA-256;
- `engine/srpm/` and `engine/rpm/`: SRPM/RPM files and raw Mock logs.

The machine stages are SRPM creation and full rebuild, not guessed RPM phase
outcomes. `build.log`, `root.log` and `state.log` retain native detail. A `Finish`
line alone is not success. Nonempty stderr alone is not failure. Successful
transport also is not a successful build: the engine verifies its returned
receipt and artifact bytes before `build` exits 0. A completed SRPM is recorded
before rebuild, so a later failure does not lose its machine-readable identity.

Without `--rm`, artifact-copy failure retains a stopped container and records
recovery commands. With `--rm`, deletion is still attempted and retrieval failure
is reported rather than silently changing the disposal policy. Cleanup failure never replaces the original failure. On Unix,
timeout terminates the host command group. A failed execution stops the worker
before retrieving evidence. Removal afterward requires `--rm`. Ctrl-C (and SIGTERM/SIGHUP on Unix) cancels
active build commands, then runs bounded stop/copy/removal and writes the failure
receipt. Further signals do not interrupt recovery. SIGKILL, host loss and daemon
failure cannot guarantee cleanup; inspect the recorded Docker project before retrying.
Artifact validation still runs when only cleanup failed, so cleanup errors cannot
hide damaged output. `clean` itself retains ordinary process signal behavior.
Report path strings are display text, not a byte-exact replay protocol.

## Supported scope and trust

The supplied target is openRuyi **x86_64**, not Fedora with renamed macros.
It uses Mock `simple` chroot isolation, no nested Docker/Podman or nspawn, and
Docker's default capability set plus `SYS_ADMIN` (not `privileged`). This grants
substantial privilege: use a disposable, trusted build host, not untrusted PRs on
a shared daemon. Compose overrides may change these properties; the actual
container inspect is retained.

The base image digest is pinned, but the openRuyi package repository is rolling
and currently configured without package signature verification. Installed
versions are recorded; exact environment replay and offline builds are **not**
claimed. `%autorelease` / `%autochangelog` follow the pinned upstream CI macro
configuration recorded in `target.json`; OBS release counters are not injected.
Artifacts are local development builds, not production release identities.

Mock 6.5's package-state plugin queries the removed RPM 6 `pkgid` tag. This target
disables that plugin; the engine validates and records a native NEVRA query
instead. Build/test success remains limited to the actual recipe and target:
disabled upstream tests and cross-architecture emulation are not native test
coverage. Do not share raw environment/log evidence without checking for secrets.
