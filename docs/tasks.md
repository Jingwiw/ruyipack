<!--
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-License-Identifier: MulanPSL-2.0
-->

# Run a package task plan

Run from an initialized workspace. Use the package plan accepted by `remote-build`:

```toml
[[packages]]
work = "ed"

[[packages]]
work = "inih"
```

```sh
ruyipack task --plan packages.toml
```

Each new task applies supported fixes, fetches materials, checks the candidate and
builds changed packages. Add `--upgrade` to request supported upgrades.
Candidate checks do not require a clean Git repository. Commit checks still apply
when you commit.

Failures retain the WORK and logs. Other tasks continue. This command does not
commit, push, open a PR, delete a WORK or repair a failed build.

## Resume or start another round

Repeat the command to resume. Unchanged tasks do not repeat fixes or builds.
Changed inputs invalidate previous validation. Use `--refresh` to check for new
fixes or upgrades. Use `--retry` after you inspect and correct a stopped task.
Retry starts with material preparation, not auto-fix.

Use `--validation local` or `--validation remote` to select the build target.
An existing task retains its target if you omit this option. Both targets use the
same WORK. Wait for a pending OBS build before changing its target or requesting
another discovery round.

## Observe OBS builds

Configure OBS authentication and project defaults first. Tasks do not prompt.

```sh
ruyipack task --plan packages.toml --validation remote --format toml
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
