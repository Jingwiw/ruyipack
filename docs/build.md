<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-License-Identifier: MulanPSL-2.0
-->

# Native development builds

`build WORK` builds the SPEC in the saved recipe directory.
It does not run `gen`, apply TOML changes or rewrite the recipe.
Static `check` requires neither Docker nor native RPM.

## One command

Install the Docker CLI and Compose plugin. Connect them to a Linux daemon.
Initialize a workspace. For an existing Git package, prepare its configured recipe repository.

```sh
ruyipack build busybox
ruyipack build busybox-test --pkgname busybox
```

The second command creates a separate development area for the same package.
A new implicit area requires a unique recipe in committed main.
Existing areas use their saved binding; they cannot be rebound.
WORK always names a development area, even when a same-named file exists.
Workspace discovery also works from subdirectories.

Default paths in `.ruyiconfig/config.toml` are:

| Setting | Default | Use |
| --- | --- | --- |
| `recipes` | `openruyi` | Recipe Git repository |
| `specs` | `SPECS` | Package directories inside the repository |
| `work` | `work` | Development areas |

All areas build `work/WORK/recipe/SPECS/PKG/PKG.spec`. The configured `specs` path selects packages in the source and commit repository.
The command does not infer paths from the RPM Name or current directory.

## Prepare materials independently

```sh
ruyipack source fetch busybox
ruyipack check busybox --materials
ruyipack build busybox --offline
```

`source fetch` checks declarations before downloading. It needs no Docker and copies committed package files when required.
Fetch and build download missing remote materials only with a valid declared SHA-256.
Local materials must already exist. Neither operation changes declarations.

Material resolution requires static Source/Patch values, not expanded build options or dependency strings.
Unrelated target macros remain uninterpreted.
Executable macros, injected declarations, includes and ambiguous material branches block resolution.
This does not establish arbitrary target macro behavior.

**`--offline` stops material downloads, not Mock dependency networking.**
Hash refresh downloads current upstream bytes and retains them by content hash.
Build can reuse those bytes after verification. `source verify` downloads independently; it does not trust this cache.

Materials come from the recipe directory and `work/WORK/sources/`.
The build copies ordinary recipe files recursively, including local patches.
Conflicting copies fail instead of selecting one silently.
An optional offline check for external materials is:

```sh
ruyipack check --spec package.spec --materials --source-dir SOURCES
```

## Results and repeated builds

Current results are in `work/WORK/build/`:

| Path | Content |
| --- | --- |
| `.config/` | Workspace assets or embedded defaults; absent with explicit `--config` |
| `input/` | Staged recipe, materials and engine program |
| `host/` | Docker/Compose logs and runtime inspection records |
| `engine/receipt.json` | Mock stages, configuration, installed packages, RPM identities and artifact hashes |
| `engine/srpm/`, `engine/rpm/` | SRPM/RPM files and Mock logs |
| `receipt.json` | Input hashes, tool identity, arguments, exit status, cancellation, runtime and cleanup results |

Named builds do not accept `--source-dir` or `--output-dir`.
Their binding selects one recipe directory and one current result.

Repeat `build WORK` after editing. Material preflight runs before archiving the previous result.
Bad materials leave that result unchanged.
Earlier results move to `work/WORK/build-history/` and consume disk until you clean them.

An unchanged environment can reuse its stopped worker and Mock dependency caches.
A repeated build still runs; an old success is not a new result.
After a shell session, the old environment stays with its archived receipt.
The next build uses a fresh worker to exclude interactive changes.
Configuration changes also require a fresh worker.

Use `inspect WORK` to list current and historical attempt IDs and resource states.
`shell WORK --attempt ID` and `clean WORK --attempt ID` select a saved attempt.
`current` selects the current build. IDs identify stored attempts, not immutable success claims.

Build output streams to stderr and remains in logs.
`--format toml` writes one outcome to stdout, linking the full JSON receipt.
Human and machine summaries share logs, accepted artifacts, `working_directory` and `next_steps` command arrays.
Run suggested commands from that directory.
A shell hint means a worker was retained; it does not prove chroot preparation succeeded.

`--timeout 1800` limits backend execution.
Bounded termination, artifact retrieval and cleanup can continue after that deadline.

## Build an external SPEC

For prepared input outside a named area:

```sh
ruyipack build --spec /path/to/package.spec \
  --source-dir /path/to/SOURCES \
  --output-dir ./build
```

This mode requires `--source-dir` and rejects `--pkgname`.
It stores results in `DIR/SPEC_STEM`; DIR defaults to `build`, and its parent must exist.
It does not create or borrow a WORK binding.
Without `--config`, it uses the embedded openRuyi environment.
Repeat the same command to archive the previous evidence and build again.

## Clean results or discard resources

Workers stop and remain available by default, including the Mock chroot.
Receipts include restart and removal commands.

Add `build --rm` to remove containers, networks and temporary volumes after retrieval.
This applies to failed builds, timeouts and retrieval failures too.
**With `--rm`, evidence that could not be retrieved can be lost.**
Host results and shared images remain.

To remove retained resources and host results:

```sh
ruyipack clean busybox
ruyipack clean busybox --force
ruyipack clean busybox --history --force
```

The first command shows the scope and asks for confirmation.
`--force` skips confirmation; it does not bypass directory or daemon identity checks.
Noninteractive cleanup requires this flag.
`--history` keeps the current result and cleans archived attempts.
Failed attempts remain available for retry, and each failure is reported.

For an explicit archived path, use `clean --build-dir PATH --force`.
A historical receipt that relinquished its worker removes logs only, never the current worker.
Cleanup uses recorded project labels, not executable commands from a receipt or changed Compose file.
A Docker connection is required even when no container remains. `clean` retains images.
`delete` also removes exclusively owned images; shared tags or other containers keep an image.
Use only trusted local receipts. Save TOML stdout elsewhere if you need a cleanup record.

Use `clean WORK --remote --force` to remove its receipt-bound OBS package instead
of local build resources. The WORK, OBS project and packages bound to other WORKs remain.
A stale remote source list blocks cleanup.

## Return to the build directory

```sh
ruyipack shell busybox
ruyipack shell --build-dir ./build/package
ruyipack shell busybox --timeout 120 -- rpm --eval '%{_builddir}'
```

`shell` restarts the retained worker and enters Mock's chroot at `%{_builddir}`.
It does not create an environment or rebuild.
The build must have reached chroot initialization without `--rm`.
Missing workers, missing build directories, ownership mismatches and running workers cause errors.

`shell` and `clean` accept WORK with `--attempt ID`, or an explicit `--build-dir PATH`.
They do not create missing areas or guess WORK from paths.
Named build and shell hold the area lock; shell also locks its receipt.
These locks coordinate RuyiPack operations, not arbitrary external writers.

A terminal gets an interactive TTY. Piped input needs no TTY, but Mock can print prompts or job-control warnings.
Interactive sessions have no deadline; `--timeout` limits probes and stopping the worker.
Arguments after `--` execute directly in the Mock build directory with a deadline.
Use an explicit shell only when you need shell syntax.
Command stdout and stderr remain separate and are saved in session logs.
`--debug` adds internal stages and log paths.

On ordinary exit, the worker stops but is not deleted.
Each session writes `host/shell-*.json`; the original build receipt remains unchanged.
Interactive streams are not recorded as a transcript.
Abrupt host termination can leave the worker running.
Shell does not provide build's cancellation recovery.

Sessions mark the environment as potentially modified, so later builds do not reuse it for verification.
Patch export alone does not mark it modified.
Receipts without explicit session tracking are treated conservatively.
The current CLI supplies the shell adapter without rewriting original build input or Mock configuration.

## Prepare, debug, export a Patch

```sh
ruyipack build busybox --stage prep
ruyipack shell busybox
ruyipack shell busybox --export-patch fix.patch --source-root busybox-1.37.0 --path applets/example.c
# Copy the Patch into the recipe directory and declare it in the SPEC.
ruyipack build busybox
```

Replace the source root and file path with those from your build.
`--stage prep` creates an SRPM and runs native prep. It is not a full build.
Successful prep saves a source baseline; full builds do not.

Export compares selected UTF-8 regular files with that baseline, including additions and deletions.
It rejects symlinks, binary files, unsafe paths and existing output files.
It does not export permissions or select generated files for you.
No source or SPEC is written back automatically. Validate the Patch with a fresh build.
`shell --timeout` bounds export; interactive shell time remains unlimited.

## Advanced configuration

Named builds copy `.ruyiconfig/build/` into the result's `.config/`.
Actual file hashes and original permissions are retained.
Embedded defaults never overwrite user workspace assets.
Keep relative asset references inside that directory.

External-SPEC builds without `--config` use the four embedded `environments/openruyi/` files.
In both default modes, only `.config/` becomes the Docker build context.
Recipe materials and logs are not image inputs.

`--config /path/to/compose.yaml` selects a custom Compose file.
Its parent is the project/build context. The file is not rewritten, and its hash is recorded.
Arbitrary referenced files are not snapshotted. Keep custom contexts small and version-controlled.

Inputs are copied rather than mounted from the client filesystem.
A remote daemon need not see local paths.
RuyiPack uses standard Docker and Compose, not a provider-specific VM interface.
It uses Docker's connection configuration unless you select `--context NAME`.
It never changes your current Docker context.
Keep personal context names and credentials in Docker configuration.

### Environment checks

Before image creation, build checks Compose, daemon identity, Linux OS and Compose configuration.
Before engine execution, it checks the resolved platform against the image and queries the worker's RPM architecture.
Temporary mount and chroot probes check required capabilities.
Mock configuration must match the target architecture.
Execution also checks Python, Mock, RPM and target configuration.

These probes do not predict repository availability, every SPEC macro or build success.
They do not install binfmt handlers or manage a VM.
They do not identify an emulator; the translator is reported as unknown.

## Engine, backend and environment

| Component | Responsibility |
| --- | --- |
| Engine (`src/build/mock.rs`, `mock.py`) | Stage execution, create SRPM, run Mock, record RPM facts and verify returned artifacts |
| Backend (`src/build/compose.rs`) | Start the worker, transfer files, execute arguments, retrieve output and clean resources |
| Environment (`environments/openruyi`) | Supply image, Compose service, repositories, macros and privileges |

The engine does not call Docker. The backend has no RPM semantics.
The authoring profile does not replace the target environment.
A new engine supplies staging, result verification and shell arguments.
A new backend supplies validation, execution, attachment and its name.
Select them at the CLI composition point. These private interfaces are not a plugin ABI.
An environment must provide the chosen engine's requirements; combinations are not automatically supported.

Compose needs a long-lived `worker` service with readable `/input` and writable `/output`.
Docker copy transfers files; no daemon socket or host source mount is required.
The build context is trusted executable configuration.

Mock needs Python 3.11+, RPM and a privilege-aware `mock` command.
`MOCK_CONFIG` identifies its configuration; `BUILD_TARGET` identifies target JSON.
The supplied image uses a normal `builder` user with a sudo wrapper restricted to Mock.
This is not isolation against hostile Mock configuration or SPEC macros.

## Evidence and failure

Engine stages record SRPM creation and full rebuild, or the requested prep stage.
Native phase detail stays in `build.log`, `root.log` and `state.log`.
A `Finish` line is not sufficient for success. Nonempty stderr is not sufficient for failure.
The engine verifies receipts and artifact bytes before build exits 0.
Completed SRPM identity is recorded before rebuild, even if rebuild later fails.

Failure categories distinguish execution, retrieval, verification and cleanup.
Cleanup failure does not replace an earlier failure or skip artifact validation.
Without `--rm`, retrieval failure retains a stopped container and recovery commands.
With `--rm`, removal is still attempted and retrieval failure remains reported.

On Unix, timeout terminates the host command group.
Failed execution stops the worker before evidence retrieval; removal requires `--rm`.
Ctrl-C, SIGTERM and SIGHUP cancel active commands, then run bounded recovery and write the failure receipt.
Further signals do not interrupt recovery. `clean` retains ordinary signal behavior.
SIGKILL, host loss and daemon failure cannot guarantee cleanup.
Inspect the recorded Docker project before retrying after such failures.
Report path strings are display text, not a byte-exact replay protocol.

## Supported scope and trust

The supplied target is openRuyi x86_64, not Fedora with substituted macros.
Mock uses `simple` chroot isolation, without nested Docker, Podman or nspawn.
Docker uses default capabilities plus `SYS_ADMIN`, not `privileged` mode.
**Use a disposable, trusted build host. Do not run untrusted PRs on a shared daemon.**
Compose overrides can change these properties; receipts retain actual container inspection.

The base image digest is pinned. The package repository is rolling and currently disables package signature verification.
Receipts record installed versions, but do not establish exact replay or offline builds.
The bundled Mock config sets `%autorelease` to `1%{?dist}` and `%autochangelog` to `%{nil}`.
`target.json` labels this development policy; the receipt retains the effective Mock config.
OBS release counters and changelog services are not validated. These are development artifacts, not production release identities.

Mock 6.5's package-state plugin queries RPM 6's removed `pkgid` tag.
The target disables it; the engine records and validates a native NEVRA query instead.
Results apply only to the tested recipe and target.
Disabled tests and cross-architecture emulation are not native test coverage.
Check logs and environment evidence for secrets before sharing them.

## Repository fallback

The profile probes primary RPM metadata before installing dependencies.
If metadata is unavailable or invalid, it probes boat and uses only that repository for the attempt.
If both probes fail, the attempt stops.
Compilation failures and missing dependencies do not trigger a switch.
Receipts record probes, selected URL and metadata digest.

Changed or unknown repository identity causes chroot and dependency-cache cleanup.
Repeated builds with the same origin can reuse caches. Shell uses the saved effective configuration.
Successful metadata retrieval does not guarantee later RPM downloads.
There is no mid-build switch or mixing of repositories.

Image bootstrap uses the same order through native DNF metadata refresh.
Only the selected repository is enabled; its URL is saved in the image and recorded when available.
For existing workspaces, add `repository_fallback` to build `target.json` explicitly and rebuild the image.
Upgrades never rewrite workspace configuration.
