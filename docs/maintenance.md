<!--
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-License-Identifier: MulanPSL-2.0
-->

# Maintain a package plan

Run from an initialized workspace. Use the same package plan as `remote-build`:

```toml
[[packages]]
work = "ed"

[[packages]]
work = "inih"
```

```sh
ruyipack maintain --plan packages.toml
```

Each package runs these existing operations in order:

1. `check --auto-fix`; add `--upgrade` to include supported upgrades.
2. `source fetch` and `check --policy submit --materials`.
3. `commit --dry-run` to check the delivery scope.
4. `build`, then `commit --dry-run --require-build`.

A package with no delivery changes stops before build. A failure retains its WORK,
command reports and logs. Other packages continue. The command does not commit,
push, open a PR, delete a WORK or repair a failed build.

## Use OBS

Configure OBS authentication and project defaults first; maintenance does not prompt.
Use `--remote` to replace the local build with an OBS submission:

```sh
ruyipack maintain --plan packages.toml --remote --format toml
```

Each invocation submits new tasks and observes due results once. Invoke the same
command from cron to continue. Waiting tasks use a 10–300 second backoff with up to
5 seconds of jitter. After 20 unsuccessful observations, the task stops. Use
`--poll-now` to ignore the next observation time.

Observation uses `remote-build --status`. It sends no remote writes. A successful
result must match the submitted source revision, service-expanded source identity
and every configured repository/architecture. Missing or dirty results wait.
Failed or stale results stop. Unavailable evidence waits within the observation
budget; it is not success.

For status without orchestration, run:

```sh
ruyipack remote-build --plan packages.toml --status --format toml
```

## Read and resume results

`--format toml` writes a report to stdout. Human progress goes to stderr in the
default mode. Exit 0 means all tasks are ready or unchanged. Exit 1 also covers
waiting tasks; inspect their phases before treating the round as a failure.

Task state and numbered command logs are under `.ruyiconfig/maintenance/WORK/`.
The report names a ready plan that contains only successful changed packages.
Each invocation clears the previous selection before it starts, then writes the
new selection. An interrupted invocation cannot leave its old selection active.

Completed tasks do not repeat auto-fix or builds. Candidate or local build
configuration changes invalidate the task. Interrupted tasks stop rather than
repeat a possibly completed external action. Inspect the retained state, correct
the cause, then use `--retry-failed` to restart at material preparation. This does
not rerun auto-fix. Changing the upgrade or backend intent requires a separate WORK.

The ready plan records results at the time of observation. It is not a release
approval. It does not yet include all upstream Action checks or a reproducible
OBS dependency environment. Commit and PR commands must retain their own input,
ownership and evidence checks. Remote success is not a local build receipt.
