<!--
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-License-Identifier: MulanPSL-2.0
-->

# Run a package task plan

Run from an initialized workspace. Define the package selection in a plan:

```toml
[[packages]]
work = "ed"

[[packages]]
work = "inih"
```

```sh
ruyipack task --plan packages.toml run
```

Use `check --plan packages.toml` to check a batch without running the maintenance chain.
Add `--auto-fix` to apply supported repairs, or `--auto-fix --upgrade` to include
supported upgrades. Failed WORKs do not stop independent WORKs. The TOML report
contains each result; any failure makes the batch fail. Duplicate WORKs are
rejected before changes. An empty plan succeeds without creating WORKs.

Single-package checks still take WORK. Task retains the existing delivery and
remote-operation batch entries:

```sh
ruyipack task --plan packages.toml remote-build --fresh
ruyipack task --plan packages.toml status
ruyipack task --plan packages.toml commit --dry-run
ruyipack task --plan packages.toml commit --require-build
ruyipack task --plan packages.toml pr
```

`commit` creates one commit per changed WORK. It reports each result and returns
failure if any WORK fails. Successful commits remain; retry does not duplicate
them. `pr` collects the selected commits into one PR without squashing them.
Neither operation starts automatically after validation.

OBS submission, status and commit accept repeated `--plan` inputs. Run and PR
require one plan so that saved selections and PR settings have one source.
Duplicate WORKs are rejected before execution.
An empty package selection succeeds without reading credentials, contacting services
or creating WORKs. It does not create commits or PRs.

Each new task fetches materials, checks existing edits and builds changed packages.
Apply repairs with `check --plan packages.toml --auto-fix` first.
Add `--upgrade` to that check for supported upgrades. Task validates the resulting
WORK files; it does not select or apply repairs.
Candidate checks do not require a clean Git repository. Commit checks still apply
when you commit.

Failures retain the WORK and logs. Other tasks continue. `task run` does not
commit, push, open a PR, delete a WORK or repair a failed build.

## Resume or start another round

Repeat `task --plan packages.toml run` to resume. Unchanged tasks do not repeat builds.
Changed inputs invalidate previous validation. After you repair a failed WORK,
use `run --retry` to resume from material preparation. A stopped task does not
resume merely because another command changed its files.

Use `--validation local` or `--validation remote` to select the build target.
An existing task retains its target if you omit this option. Both targets use the
same WORK. Wait for a pending OBS build before changing its target or requesting
a new validation round.

## Observe OBS builds

Configure OBS authentication and project defaults first. Tasks do not prompt.

```sh
ruyipack task --plan packages.toml run --validation remote --format toml
```

Each invocation observes due results before it starts new builds. Invoke it from
cron to continue. Observations use a 10–300 second backoff with up to 5 seconds of
jitter. Queued builds have no observation-count limit. Twenty consecutive query
failures pause the task. Use `--poll-now` to ignore the next observation time.

A passed result must match the submitted revision, expanded source identity and
all configured targets. Stale evidence pauses the task. Failed builds stop it.
Previously validated remote results are checked again on the next invocation.

## Read results

`--format toml` writes the report to stdout. Default progress goes to stderr.
Exit 0 means that the round ran without a failed or paused task. A task can still
be waiting: inspect its phase before starting a dependent operation.

State and numbered command logs are in `.ruyiconfig/tasks/WORK/`.
Each completed round writes its own report and `validated.plan.toml`.
The selection contains only changed packages with matching validation evidence.
An interrupted round does not replace a previous selection with an empty one.
A saved selection is a historical result, not current approval.

Commit and PR commands still check their own inputs and ownership. This task
chain does not cover all upstream Action checks or guarantee reproducible OBS
dependencies. Remote success is not a local build receipt.

## Delete a plan's WORKs

```sh
ruyipack task --plan packages.toml delete --dry-run --format toml
ruyipack task --plan packages.toml delete --force --format toml
```

The plan selects WORKs; it does not change deletion ownership checks. Each WORK has a result.
A failed deletion does not stop independent WORKs. Cancellation stops the batch
and reports pending WORKs. OBS projects, shared images and the plan file remain.

Commit and delete batches report `results` and `pending`. Each result describes
one attempted WORK. `pending` contains WORKs not attempted after cancellation.
A failed result does not imply that its earlier actions were rolled back.


## Require a directory check

Prepare a directory with the selected WORK SPECs under `SPECS/PKG/` and the
target repository's `scripts/` and `.pre-commit-config.yaml`. Then run:

```sh
ruyipack check --directory prepared --check-output checks
ruyipack task --plan packages.toml run --check-receipt checks/receipt.toml
```

The receipt requirement is saved per WORK. Later runs still check it when the
option is omitted. A failed or stale result excludes that WORK from the validated
selection. Correct it, run the directory check into a new output directory, then
pass the new receipt with `run --retry`.

Pre-commit runs once for all prepared package files, in a disposable Git index.
It does not need a publication repository. A pre-commit failure blocks the
selection; hook edits stay in the disposable copy. Logs and a diff are retained.
After it passes, each WORK needs its own successful RemoteAsset result.
Another package's RemoteAsset failure does not reject it.

Incomplete execution rejects the receipt. Package files, hook scripts, pre-commit
configuration, the Compose file and the driver must still match. Hooks that depend
on repository history can behave differently in the disposable index. This does
not claim complete GitHub runner parity.
