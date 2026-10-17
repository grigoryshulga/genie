---
title: Workspace environments design
summary: Proposed model for reproducible, isolated per-task environments across several repositories.
type: note
status: draft
---

# Workspace environments design

This is a proposal, not implemented behavior. [Project environments](environment.md) describes what Genie supports today.

A project may combine independent repositories: frontend, backend, integrations, and infrastructure. Each task needs a complete environment that can build, test, and run that combination, while several tasks run in parallel. Prepared dependencies should be reused physically without letting one task affect another.

The core manages environment lifecycle, storage, policy, and recovery. Knowledge of languages, package managers, and frameworks lives in recipes. **A new stack must be supported by adding a recipe, without changing the core.**

## Starting point

The current implementation already provides:

- Several repositories per project, each with a mount path and access policy ([repositories](git-repositories.md)).
- Bare mirrors fetched only by the server, and per-workspace clones at `<data>/workspaces/<project>/<name>/` made from them with hard-linked objects. A clone's `origin` is the Git proxy; hosting tokens stay on the server.
- Per-task repository scope, task branches, and per-repository delivery records with request state, head SHA, and CI state.
- A bubblewrap sandbox: a read-only system, a writable workspace, shared tool caches, a shared network, and a read-only `.git/config` and hooks.

What is missing is a pinned toolchain, system packages, services, resource limits, network policy, and isolated reuse of installed dependencies. Shared writable tool caches are also a cross-task integrity risk.

## Domain model

| Entity | Meaning | Owner |
| --- | --- | --- |
| **Project** | Repository catalog, environment profiles, and access policies. Exists today. | Project administrators |
| **Environment profile** | What a workspace needs: base, components with recipes, services, commands, checks, and requested capabilities. A project can have several, for example `default` and `integration`. | Project administrators |
| **Recipe** | A reusable, versioned description of one stack: tools, preparation steps, caches, snapshot inputs, and commands. Built-in or project-defined. | Server or project |
| **Runner** | An executor that creates environments and enforces limits. Declares its capabilities. | Server administrators |
| **Workspace** | One task's environment: the selected repositories, their working copies, prepared dependencies, services, and mutable data. | Task |
| **Workspace lock** | The resolved inputs of a workspace: repository SHAs, base digest, tool versions, recipe revisions, snapshot keys, and service image digests. | Generated |
| **Snapshot** | An immutable prepared-dependency tree published under its key. | Generated, shared |
| **ChangeSet** | The task's related requests across repositories, their head SHAs and checks, delivery order, and integration results. | Task |

Relationships:

- A task has at most one active workspace per workspace name; today the name is a team id or `job-<n>`. Teams that should share files keep sharing one workspace.
- A workspace uses one profile and one runner. Its lock is written when preparation starts and amended only by explicit refresh.
- A snapshot is referenced by any number of workspaces. Workspaces never write to it.
- A ChangeSet extends the existing per-repository delivery records rather than replacing them.

## Profiles and recipes

### Recommended format and location

Use JSON files validated against a published schema, rejecting unknown fields, like `git.json` and team templates. This keeps one configuration format and lets the web edit the same files ([PD13](decisions.md)).

- Built-in recipes ship in `config/environments/recipes/<id>.json` and are versioned with the release.
- Project profiles and project recipes live in server data under the project. They are edited by project administrators in Settings and by the CLI.
- A profile may reference files in a repository, such as a bootstrap script, `mise.toml`, a `Containerfile`, or `devcontainer.json`. Their content at the locked SHA becomes a snapshot input. A repository can therefore evolve its setup through normal review, while the capabilities it requests stay under server policy.

A repository-only profile that grants itself network, secrets, or mounts is deliberately not supported.

### Recipe contract

A recipe declares:

| Section | Content |
| --- | --- |
| `id`, `revision` | Identity. The revision and a hash of the file enter every key that uses it. |
| `params` | Typed parameters with defaults, such as the Node version or a subdirectory. |
| `tools` | Tool versions, resolved to exact versions in the lock. Provided by the base or installed by a step. |
| `caches` | Download caches: path inside the environment, scope, and whether the package manager verifies content integrity. |
| `prepare` | Steps producing prepared dependencies: command, output paths, inputs, environment variables, network and secret needs, and a verification command. |
| `commands` | Named commands such as `build`, `test`, `lint`, and `run`. |
| `requires` | OS, architecture, resources, and special capabilities. |

Example built-in recipe:

```json
{
  "id": "node-npm",
  "revision": 1,
  "params": { "node": { "default": "22" }, "dir": { "default": "." } },
  "tools": { "node": "{{node}}" },
  "caches": [{ "name": "npm", "path": "~/.npm", "scope": "project", "integrity": "verified" }],
  "prepare": [{
    "id": "install",
    "run": "npm ci",
    "cwd": "{{dir}}",
    "outputs": ["{{dir}}/node_modules"],
    "inputs": ["{{dir}}/package.json", "{{dir}}/package-lock.json", "{{dir}}/.npmrc"],
    "env": ["NODE_ENV"],
    "network": "registries",
    "verify": "npm ls --omit=dev"
  }],
  "commands": { "build": "npm run build", "test": "npm test" }
}
```

A recipe that cannot enumerate its inputs uses `"inputs": "repository"`. The key then includes the Git tree of the whole repository, which is correct but rarely reused. The `custom` recipe uses this default, so an arbitrary bootstrap script is safe before it is tuned.

### Adapters

Some stacks need logic: detecting workspaces in a monorepo, reading a Gradle settings file, or translating `devcontainer.json`. An adapter is an executable that runs inside the environment and prints a recipe fragment as JSON on standard output. The core treats its output as data. Adding an adapter is adding a file, not changing the core.

### Profile

```json
{
  "base": { "image": "docker.io/library/debian:12", "packages": ["build-essential", "libpq-dev"] },
  "requires": { "arch": ["x86_64"], "memory": "8GiB", "cpus": 4 },
  "components": [
    { "repo": "web", "recipe": "node-npm", "params": { "node": "22" } },
    { "repo": "api", "recipe": "rust-cargo" },
    { "repo": "legacy", "recipe": "custom", "prepare": [{ "run": "./scripts/bootstrap.sh" }] }
  ],
  "services": {
    "db": {
      "image": "docker.io/library/postgres:16",
      "env": { "POSTGRES_PASSWORD": "genie" },
      "ready": { "command": "pg_isready -U postgres" },
      "data": "workspace"
    }
  },
  "processes": {
    "api": { "repo": "api", "command": "cargo run", "after": ["db"], "ready": { "http": "http://127.0.0.1:8080/health" } }
  },
  "checks": { "integration": "make -C infra e2e" },
  "network": { "prepare": "registries", "work": "none" },
  "secrets": { "npm_token": { "steps": ["web/install"], "file": "/run/secrets/npm" } }
}
```

The base is either an image plus packages, or a `Containerfile` from a repository. Both resolve to an image digest in the lock.

## Runner contract

A runner exposes operations, all idempotent by a caller-supplied identifier:

| Operation | Effect |
| --- | --- |
| `capabilities` | Static and probed facts: OS, architecture, isolation features, storage modes, network modes, enforceable limits, services, devices. |
| `ensure_base` | Build or pull the base and return its digest. |
| `create` / `remove` | Create or delete the environment for a workspace id. Removal never touches source clones. |
| `exec` | Run a step with mounts, environment, secrets, limits, network mode, and a timeout; stream logs; return the exit status. |
| `start` / `stop` | Start or stop services and processes; stopping keeps all data. |
| `probe` | Run a readiness check. |
| `list` | Every resource the runner holds that is labelled with a Genie workspace id. Used for reconciliation. |
| `usage` | Logical size and exclusively owned physical size per workspace and snapshot. |

Before creating a workspace, Genie intersects three sets: what the profile requires, what server policy allows for the project, and what the runner supports. Any gap fails before work starts, with every unmet item listed, for example `memory limit: runner cannot enforce (cgroup v2 controller "memory" not delegated)`. A requirement that cannot be enforced is never downgraded silently.

### First runner

Use **rootless Podman** on Linux:

- Rootless user namespaces, seccomp, separate PID and network namespaces.
- Resource limits through delegated cgroup v2 controllers.
- Pods for services with a private network per workspace.
- OCI images for bases and services, pinned by digest.
- Native rootless overlay mounts on current kernels, which provide the copy-on-write view of snapshots.

The existing bubblewrap mode becomes a `host` runner with a reduced capability set: no base image, services, or enforced limits. Profiles that need these fail the compatibility check instead of degrading. Docker deployment of Genie needs either a remote runner on the host or a nested rootless runtime; this is a deployment decision, not a contract change.

VM and remote runners implement the same operations over RPC. Snapshot and cache storage belongs to the runner's host, so keys are scoped by runner storage.

### Where the agent runs

Today the agent harness runs on the host inside bubblewrap. With an environment runner, the agent's commands must run inside the environment. Recommendation: run the harness process inside the workspace environment, with Genie's harness runtime mounted read-only, and allow network only to the Genie API and model endpoints in addition to the profile's work policy. File edits and commands then share one isolation boundary. Bridging only shell commands into the container would leave file tools and commands with different views.

## Storage and reuse

The environment store lives outside Git directories, under `environments.root` (default `<data>/environments`), preferably on a filesystem with reflink support:

```text
<env-root>/
  caches/<scope>/<name>/        download caches
  snapshots/<key>/              immutable prepared dependencies
  snapshots/.staging/<key>.<n>/ preparation in progress
  workspaces/<id>/upper/        private changes on top of snapshots
  workspaces/<id>/build/        build outputs and temporary data
  workspaces/<id>/services/     service data
```

Source clones stay at `<data>/workspaces/<project>/<name>/`. Base images live in the runner's image store.

| Category | Mutability | Sharing | Retention |
| --- | --- | --- | --- |
| Base: tools and system libraries | Immutable, by digest | Server-wide | Kept while referenced by a lock; then LRU |
| Download cache | Append-mostly | Per project by default | Size-limited LRU; always recoverable |
| Prepared dependencies | Immutable snapshot plus private writable view | Any workspace with the same key | Pinned while referenced; then LRU |
| Workspace data | Mutable | One workspace | Per policy: ephemeral, until deletion, or persistent |

### Paths

Every environment mounts its repositories at the same canonical path, for example `/workspace/<mount>`, and uses the same home directory. Absolute paths written during installation, such as virtualenv shebangs or native module paths, are therefore valid in every workspace that uses the snapshot. This is the main reason to prefer a container runner over host directories with varying paths. A runner that cannot provide canonical paths must include the real path in the snapshot key.

### Independent views

Each output path of a prepare step is mounted into the workspace as a writable view of the snapshot:

1. **Overlay**: the snapshot is the lower layer; the workspace's `upper/` receives changes. Deleting or upgrading a package affects only that workspace.
2. **Reflink copy**: on Btrfs or XFS, a clone shares extents until written.
3. **Full copy**: always available; slower and larger.

Genie selects the first mode the runner supports and shows it on the workspace. Hard links to shared files and a shared writable dependency directory are not views: a write through one workspace would change the others.

### Download caches

A download cache shared between concurrent untrusted processes can be poisoned. The default scope is therefore the project, which is already a trust boundary. A cache may be shared server-wide only when its package manager verifies content against lock-file hashes, such as Cargo, npm, and uv. Caches are writable during preparation and read-only during work; a workspace that installs more during work gets a private overlay of the cache.

### Disk accounting

For every workspace and snapshot, record the logical size and the physically exclusive size: the overlay upper directory, unshared extents for reflink copies, or the full size for copies. Acceptance tests use these figures to show reuse.

## Snapshot keys

A snapshot key is the SHA-256 of a canonical JSON document containing:

- Recipe id, revision, and file hash; the step definition after parameter substitution.
- The base digest, OS, architecture, and the runner's path layout.
- Resolved tool and package-manager versions.
- The content hashes of the declared inputs, read from the locked commit rather than the working tree.
- Declared environment variables and install flags.
- Keys of the snapshots this step depends on.

A lock-file change therefore selects a different key. When inputs are unknown, the key includes the repository tree; an operator can also force a fresh preparation.

### Publication

1. Acquire a lease on the key in the server database. Concurrent requests for the same key wait for this single preparation.
2. Prepare into `.staging/` with a fresh directory per attempt.
3. Run the step's verification. Record the file manifest and size.
4. Make the tree read-only, rename it into `snapshots/<key>/`, then mark it `ready` together with its event. A published directory without a `ready` record, left by a crash between these steps, is checked against its manifest on recovery and either marked ready or removed.

Readers use only `ready` snapshots. A crashed or failed attempt leaves a staging directory and an expired lease; both are reclaimed by the next preparation or by cleanup. An empty store is the normal starting state.

## Isolation and policy

Server policy defines what a project may receive; a profile requests a subset. The effective grant is their intersection, and so is enforcement:

| Control | Policy grants | Enforcement |
| --- | --- | --- |
| Mounts | Source clones, views, caches, and build directories only. No host paths except those allowed by administrators. | Runner mounts |
| Network | Per phase: `none`, `registries` (an allowlist of package hosts), or `full`. Git traffic always uses the proxy. | Network namespace, egress proxy |
| Secrets | Named server secrets for named steps, delivered as files on a temporary filesystem. Hosting tokens are never delivered. | Runner |
| Resources | CPU, memory, processes, disk, and step timeouts. | cgroup v2 and quotas, or measurement with eviction when quotas are unavailable and the policy allows it |
| Git | Existing role, task scope, and delivery policy through the proxy. | Git proxy |

Commands from repositories, including dependency install scripts, run only inside the environment. If a mandatory control is unavailable, the operation fails with an actionable error.

The first release supports `none` and `full` network modes. The `registries` allowlist needs an egress proxy and follows in a later phase; until then, profiles that request it fail the compatibility check.

## Lifecycle and recovery

Workspace states:

```text
requested → sources_ready → environment_ready → services_ready → ready
ready ⇄ stopped
any stage → failed(stage)    failed → retry from that stage
stopped | failed → deleting → deleted
```

Each stage is a persisted step with an idempotency key made of the workspace id, the stage, and its input hash, following the durable step pattern used by automations. A completed stage with the same key is not repeated.

Recovery after a restart:

1. Load workspaces that are not `deleted`.
2. Call `list` on each runner and match resources by workspace label.
3. Adopt matching resources, mark missing ones for the stage that should have created them, and stop unlabelled or orphaned ones after a grace period.
4. Expire leases held by the previous process and clean their staging directories.

Stopping releases processes and services and keeps everything. Deleting removes environment resources; source clones are deleted only when they have no uncommitted changes and no commits missing from the remote. Otherwise deletion requires a person's confirmation.

### Cleanup

Active and stopped workspaces pin their base, snapshots, and service images. Cleanup removes unpinned items in LRU order when a size limit is exceeded, then trims download caches. It never removes a pinned item and never deletes unpublished source work. Reproducing tools and preserving user work are separate concerns: tools are rebuilt from the lock, but source changes must reach a remote or a backup.

## Several repositories and delivery

A workspace places the selected repositories at their mounts, as today: write access on the task branch, read access on the default branch or a pinned SHA. The lock records each SHA.

The **ChangeSet** of a task is built from its per-repository delivery records:

- Each member has a repository, branch, request, observed head SHA, and checks; these already exist.
- Integration results are recorded for an exact combination of head SHAs. Any head change invalidates the result.
- Optional delivery edges state that one repository's change must merge before another's. Merge gates show or enforce the order according to delivery policy.

An integration check creates a workspace from the ChangeSet's head SHAs using the `integration` profile, runs its `checks`, and attaches the result and lock to the task. Atomic merges across repositories are not attempted; contract compatibility and order stay explicit.

## Decisions to confirm

| Question | Recommendation |
| --- | --- |
| Profile format and location | JSON with a schema in server data, edited in Settings and the CLI; repository files referenced and hashed. |
| Entity names | Keep the names above. Use `environment` for the profile in user-facing text and `workspace` for the per-task instance. |
| Runtime and copy-on-write | Rootless Podman; overlay views first, reflink then copy as fallbacks. |
| Worktrees or clones | Keep clones from mirrors. Clones are already cheap with hard-linked objects, and separate `.git` directories keep configuration and hooks independent. Linked worktrees share them, which weakens isolation. |
| Host builds outside an environment | Keep only the `host` runner with its reduced capabilities for projects without system requirements. No separate unmanaged mode. |
| Agent placement | Run the harness inside the environment. |
| First release | See below. |
| Network, secrets, quotas | Defaults: prepare `full`, work `none` unless granted; no secrets unless named; per-workspace memory 8 GiB, 4 CPUs, 4096 processes, 50 GiB disk, all adjustable by policy. |

## First release

Included:

- Domain records, profile and recipe schema, and compatibility checks.
- Rootless Podman and the existing host runner.
- Base images from an image or a `Containerfile`, pinned by digest.
- Built-in recipes: `rust-cargo`, `node-npm`, `python-uv`, `jvm-gradle`, and `custom`.
- Snapshots with keys, single-flight preparation, verification, overlay and copy views, and disk accounting.
- Project-scoped download caches.
- Services from images with readiness probes; processes started from repositories.
- Network modes `none` and `full`; named secrets as files; CPU, memory, and process limits.
- Persisted lifecycle, reconciliation, and pinned LRU cleanup.

Deferred: registry allowlists, reflink views, server-wide caches, VM and remote runners, devcontainer adapter, ChangeSet delivery ordering, and integration checks across repositories.

## Implementation order

1. Domain model, schemas, runner trait, and compatibility checks, with tests on fake runners.
2. One vertical path on a single repository: create, prepare, build, run, stop, delete.
3. The environment store, download caches, snapshots, and independent views.
4. Recovery, leases, reconciliation, and safe cleanup.
5. Recipes for Genie (Rust and npm), a Python project, and a JVM project.
6. Several repositories, services, and ChangeSet integration checks on a multi-repository system with a database.

## Acceptance scenarios

| Scenario | Evidence |
| --- | --- |
| Two workspaces build and run at the same time. | Both reach `ready`; their commands pass; their ports and data are separate. |
| Identical dependencies are reused. | The second workspace uses the same snapshot; its exclusive size excludes the snapshot. |
| Changing or deleting a dependency in one workspace leaves the other intact. | A removed package is still present and the snapshot is unchanged. |
| A lock-file change selects another environment. | A new key and snapshot; the old snapshot remains. |
| A failed install never appears ready. | The key is not `ready`; staging is reclaimed. |
| A restart restores control. | Reconciliation adopts resources and creates no duplicates. |
| Cleanup preserves active environments. | Pinned snapshots survive a forced cleanup under a low limit. |
| An empty store can be rebuilt. | Deleting caches and snapshots, then preparing again, succeeds. |
| Incompatible requirements fail early. | A profile requesting an unavailable capability fails before any step runs. |
| A new stack needs only a recipe. | A recipe added in project configuration works without a Genie release. |
| Multi-repository changes are checked at fixed SHAs. | An integration result names the exact SHAs and becomes stale when one changes. |
