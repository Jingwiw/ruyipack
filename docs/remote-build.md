# OBS remote builds

`remote-build` submits the current WORK SPEC and materials. It does not generate a SPEC, upgrade a version, commit Git changes, or claim that queued builds passed.

## Authentication and defaults

Set `.ruyiconfig/obs-auth.toml` with mode `0600`:

```toml
user = "YOUR_USER"
password = "YOUR_PASSWORD"
```

The first interactive invocation can ask for these values and save this file. RuyiPack adds it to `.ruyiconfig/.gitignore`. Do not commit credentials already tracked by Git. RuyiPack does not read osc credential files.

`.ruyiconfig/obs.toml`:

```toml
api = "https://build.openruyi.cn"
parent = "openruyi"
repositories = ["x86_64"]
publish = false
```

Select parent repositories, not only CPU names: `rva20` and `riscv64` can both use the `riscv64` architecture. The parent metadata supplies the actual architectures. A parent project URL such as `https://build.openruyi.cn/project/show/openruyi` is also accepted.

## One package

```sh
ruyipack remote-build ed
ruyipack remote-build ed --fresh
ruyipack remote-build ed --format toml
```

The first invocation asks for project, parent repository choices, and RPM publication. Settings are saved in `work/ed/remote.toml`. Later invocations show remote status; `--fresh` submits the changed file set. Import settings with `--from-config work/OTHER/remote.toml`.

Projects must be below `home:YOUR_USER:`. Existing projects without the RuyiPack ownership marker are not replaced. A remote package changed outside this WORK is not overwritten. Source uploads use OBS's file-list commit protocol; a failed upload does not publish a partial file list. A lost commit response requires inspecting the remote state before retrying.

## A plan

```toml
project = "home:YOUR_USER:package-validation"
parent = "default"
repositories = ["x86_64"]
publish = false

[[packages]]
work = "ed"

[[packages]]
work = "atf"
```

```sh
ruyipack remote-build --plan plan.toml --format toml
ruyipack remote-build --plan plan.toml --fresh --format toml
ruyipack remote-build --plan basic.plan.toml --plan upgrade.plan.toml \
  --repositories x86_64,riscv64,rva20
```

`--plan` can be repeated. `--repositories` updates the existing projects and saves resolved WORK settings without requiring `--fresh` or uploading retained materials. It replaces the enabled repository list, so include every repository to retain. Plans remain user-owned inputs: update their repository lists before reusing them without the override.

Omitted values inherit workspace defaults; `parent = "default"` explicitly selects the default parent. Each task saves its resolved settings in its own WORK. All tasks sharing a project must agree on project settings. A missing WORK can be prepared from the configured recipe repository. Tasks run sequentially, with separate errors and receipts. Retry completed tasks without `--fresh` to read their results rather than uploading again.

## Observe submitted builds

```sh
ruyipack remote-build --plan plan.toml --status --format toml
```

`--status` reads existing WORK settings and submission receipts. It does not create
WORKs, change local settings, upload files or change OBS projects. It queries each
project once for its target configuration and once for results, then checks each
package source revision. Plan settings do not override retained bindings in this mode.

A target passes only when OBS reports `succeeded`, the result is not dirty, and
its build revision and source identity match the submitted files. A completed
source service supplies the expanded build identity; a running or failed service
cannot pass. Every configured
repository/architecture must pass. Missing targets remain waiting; missing build
identity is unavailable, not success. Failed, stale and unavailable observations
remain separate. Exit 0 requires all selected tasks to pass.

This reports source and target-matrix evidence, not reproducible dependency
snapshots or PR approval. The caller must also check local candidate identity.

## Source services and evidence

Local submission does not use `obs_scm`: pulling Git could replace local changes. The default `_service` runs `download_assets` in `trylocal` mode. Set `.ruyiconfig/obs-service.xml` for a shared template, `work/WORK/_service` for a local override, or `service = "PATH"` in remote settings (relative to `.ruyiconfig`, or absolute). A recipe's `_service` is also used when no WORK override exists. Custom services execute on OBS; review them before submission.

Remote HTTP materials are downloaded and checked against declared SHA-256 before submission, using the same preparation as local builds. OBS receives the complete regular files in the recipe directory, declared cached materials, and `_service`. Hidden files are excluded; subdirectories, symlinks, conflicting names and reserved OBS control files are refused.

`remote-receipt.toml` records submitted filenames, SHA-256, OBS protocol MD5 and revision. MD5 is used only for the OBS protocol, not material authenticity. `remote-result.toml` records submission and build states. A successful submission is not a successful build: `scheduled`, `building`, `blocked`, `unresolvable`, `failed`, and `succeeded` remain distinct. RPM publication is disabled by default. Remote projects and packages are not deleted by local `clean` or `delete`.

### OBS constraints and Git publication

Set `constraints = "obs-constraints.xml"` in OBS settings to read a file relative
to `.ruyiconfig`. Plan and WORK settings can override this path. Without this
setting, remote builds read `WORK/_constraints`, if present. A package-local
`_constraints` is also accepted; conflicting copies stop submission.

`commit` and `pr` exclude package-relative paths listed in
`.ruyiconfig/commit-ignore.toml`. The default is:

```toml
files = ["_constraints"]
```

Entries are exact paths, not glob patterns. Exclusions do not affect OBS uploads.
Local files remain intact. Files already in Git or a pending commit require a
separate reviewed removal. A refresh does not silently remove remote constraints.

Add package reminders when publishing a PR:

```sh
ruyipack pr --plan packages.toml --note 'mypackage: Tests need ICMP socket permission.' --publish
```

Repeat `--note 'PACKAGE: TEXT'` for more reminders. Each reminder must be one
line and name a package in the plan. Reminders are added to the generated
summary, below their package; they are not saved in the plan. Repeat the options when updating the
PR if the reminders still apply.
