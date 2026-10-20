---
title: Visual review before publishing
summary: Prepare combined local previews and obtain owner approval before publishing task branches.
type: guide
status: current
---

# Visual review before publishing

An agent prepares a committed, validated task and a fixed integration candidate, then waits for the owner to approve that candidate and authorize publication. Feature PRs target `dev`; only release PRs from `dev` target `main`. [AGENTS.md](https://github.com/grigoryshulga/genie/blob/main/AGENTS.md) defines the mandatory gates, and [the tracker guide](genie-cli.md) defines task commands.

## Local installation

The portable code and tests live in `scripts/preview/`. Install a trusted, reviewed checkout as the ordinary preview owner:

```bash
python3 scripts/preview/install.py
```

The installer discovers the repository's shared Git directory and installs versioned code under `$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/`. Stable launchers there are shared by every worktree; running workers retain their installed code version when a newer version is installed. The `prepare` action creates an excluded `.genie-preview` symlink in the current worktree. Installing does not change T3 Actions, restart services, push a branch, or publish a PR.

Only source code, tests and the safe [example configuration](https://github.com/grigoryshulga/genie/blob/dev/scripts/preview/config.example.json) enter Git. Machine settings, generated service units, dependencies, logs, databases, credentials, Tailscale identity and build artifacts stay in the shared Git directory. Never publish that runtime directory. The helper requires Linux, Python 3.12+, Git, Node/npm and the repository's Rust toolchain; optional LAN publishing requires systemd, Avahi and `iproute2`, and remote access requires Tailscale.

### Minimal configuration

Local previews need no configuration parameters. Installation creates `local.json` from `scripts/preview/config.example.json`:

```json
{
  "lan": {
    "interface": "auto"
  }
}
```

An empty object `{}` also uses all defaults. For optional LAN setup, automatic discovery accepts exactly one IPv4 default-route interface, excludes known tunnel/virtual interfaces, and requires exactly one private global IPv4 address. With a VPN, multiple default interfaces or multiple addresses, it stops with an actionable error. Set only `lan.interface` to the intended interface (for example `eth0`) when the interface choice is ambiguous; an interface with multiple IPv4 addresses needs its address configuration resolved before installation.

The installer computes the username/group, absolute paths, executable locations, selected interface's IPv4 address, current user's runtime socket and the original Tailscale node's IPv4 address. `enable-tailscale` and `enable-services` discover their respective MagicDNS names from the running daemons. A copied config never supplies a username, UID, IP or tailnet domain.

Optional advanced overrides in `local.json` are `baseRef`, `indexPort`, `httpsPort`, `candidatePortStart`, `candidatePortEnd`, `backendPortOffset` and `buildJobs`; defaults and validation live in `scripts/preview/config.py`. LAN names, Service names and router ports are shared project conventions. `config.json` and `services/config.json` hold discovered network state and are managed by the helper, rather than copied between machines. Existing installations retain discovered URLs and private state; legacy non-default settings migrate into `local.json` during installation.

Reinstall after reviewing changes to the tracked helper. Start a fresh index process and restart routers to use the new code; existing candidate artifacts and URLs remain fixed. Changing listen ports or addresses requires regenerating/reinstalling units and matching Tailscale mappings, then checking real URLs. Old candidates retain their original ports.

From a task worktree, define this convenience function for the examples below:

```bash
preview() {
  python3 "$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/preview.py" "$@"
}
```

## T3 Project Actions

Create two project-scoped Actions. Their commands discover the shared git directory from the thread's current worktree; no machine-specific absolute path is required.

**Prepare preview workspace** command:

```bash
python3 "$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/preview.py" prepare
```

Enable **Run automatically on worktree creation** and **Wait for it to finish before the agent starts**. Leave **Run when the thread settles** and automatic preview opening disabled. Preparation installs exact npm dependencies only when the lockfile/manifests changed, reuses npm's download cache, and starts the shared preview index. Installed dependencies remain worktree-local.

**Request integration preview** command:

```bash
python3 "$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/preview.py" request
```

Enable **Run in the thread's worktree when the thread settles** and **Open preview automatically when this action runs**. Leave creation-time execution disabled. Set **Preview URL** to `https://genie-preview.<tailnet>.ts.net/__previews` after the Services setup below, or use `http://genie.localhost:7480` for local-only use. The helper prints the current verified index URL; do not hardcode a previous tailnet name. This Action queues work and returns; compilation runs independently of the thread.

An interruption, question or error can settle a thread. The request Action does nothing unless the worktree has explicitly marked its current clean commit ready. Configure the Actions manually; installing the helper does not modify T3 project settings.

## Mark a task ready

Run applicable checks, inspect the full diff, commit only this task's changes, then:

```bash
preview ready --task GNE-123
preview request
preview status
```

Readiness records the task ID, branch, worktree and exact commit. The helper never commits, pushes, creates PRs, approves tasks, or infers readiness from a branch name. Tracked or untracked changes, a different HEAD, or a removed worktree invalidate readiness. Ignored dependency and build directories are not task changes.

Before resuming edits or removing a task from the next composition:

```bash
preview withdraw
```

Other valid ready tasks remain selected. If any declared task is stale, the combined build fails with a named error instead of dropping that task. Mark it ready again or withdraw it before requesting another build.

## Combined candidates

One worker at a time merges all explicitly ready task commits, sorted by task ID, into a dedicated detached worktree from the current local `origin/dev`. Fetch `origin/dev` before declaring readiness; builds themselves do not fetch or move the base. A conflict blocks the entire declared composition. Agents' worktrees remain unchanged.

The candidate ID is derived from the base SHA and ordered task IDs/SHAs. Duplicate requests reuse the same running candidate. A request received during another build waits for the build lock, then evaluates the newest selection. The worker checks readiness again before publication. The index advances only after successful build and startup checks; failed candidates do not replace the previous working preview.

Each candidate retains its own binary, static web assets, integration worktree and database. Source versions behind a candidate link do not change. Review interactions can change its disposable database; it is not a read-only data snapshot. Build caches are shared only by the serialized builder. Backend artifacts are keyed by backend sources and compiler identity; UI-only changes reuse that backend, while web assets are rebuilt from the exact combined source. Rust uses the development profile for local review, not a release optimization build.

The backend runs with `--no-agents`, its own home/data directories, no inherited tracker credentials or model keys, and a seeded manual project. These previews cover UI/API interaction, not live agent/provider behavior. A private `credentials.txt` next to the candidate manifest contains the generated `preview` account password; read it locally without putting it into task comments or logs. Supply additional representative test data in the disposable preview when a review scenario needs it.

The index exposes `/api/status` with candidate manifests and readiness, not passwords, databases or filesystem browsing. Candidate gateways keep authentication cookies separate even when multiple candidates use different ports on the same Tailscale hostname. Browser API requests and event streams stay on the candidate's origin.

## Local names and Tailscale

Local index: `http://genie.localhost:7480`. Candidate application ports are allocated from 8500 through 8599. Local browsers resolve `.localhost` to loopback; on a remote machine that points to the remote machine itself. Use the Tailscale URL remotely.

Enable remote access once on the build host:

```bash
preview enable-tailscale
```

The command discovers the machine's current MagicDNS name, publishes the loopback index through Tailscale Serve on HTTPS port 8443, and prints both addresses. Each ready candidate gets its own HTTPS port matching its local gateway port. Tailscale access policy must permit the reviewing devices to access those ports. This uses private tailnet access, not Funnel. Existing Serve listeners, including T3 on port 443, are preserved; a conflicting port is an error.

When the tailnet or machine name changes, run the command again before requesting new candidates. The index reloads its host configuration on requests. Use the newly printed remote URL in the T3 Action. Existing candidate manifests retain the address used when they were created; after a DNS rename, restart and verify links rather than assuming an old hostname still resolves.

Candidate backends and gateways bind only to loopback. The gateway forwards to the candidate's private backend and preserves separate local/Tailscale entry points. Connecting to an index through Tailscale yields Tailscale application links, so remote navigation does not fall back to loopback. The optional LAN router below adds explicitly selected listeners.

## LAN names and baseline environments

The optional `lan.py` helper publishes these mDNS names through the existing Avahi daemon:

- `http://genie.local`: isolated snapshot of `origin/main`.
- `http://dev.genie.local`: isolated snapshot of `origin/dev`.
- `http://preview.genie.local`: latest successfully published integration candidate.
- `http://<candidate-id>.preview.genie.local`: a fixed candidate for approval.
- `http://preview.genie.local/__previews`: the environment index with fixed candidate links.

The short environment names can advance to a new candidate. Use the candidate ID hostname or fixed Tailscale application URL for approval. No baseline reuses the live tracker's database. To queue a refreshed baseline after fetching its branch:

```bash
git fetch origin main dev
preview channel main
preview channel dev
```

Generate the LAN service units from this machine's discovered settings:

```bash
python3 scripts/preview/install.py --lan --install-user-units
preview_home="$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview"
```

Review `"$preview_home/units/system/"` before the administrator step. Use `sudo` from an interactive terminal (`pkexec` when a graphical authentication prompt is needed):

```bash
sudo python3 "$preview_home/code-current/install_network.py" \
  --units "$preview_home/units/system" --mdns
sudo systemctl enable --now genie-local-proxy.socket
sudo systemctl restart genie-local-proxy.service
systemctl --user enable --now genie-mdns.service
```

The generated `genie-mdns.service` keeps aliases published and follows the selected LAN interface's IPv4 address. The `genie-local-proxy.socket` system unit owns port 80 on discovered loopback, LAN and original Tailscale IPv4 addresses; its service runs as the discovered ordinary user. A running Tailscale daemon is optional for LAN installation, but an installed, inaccessible daemon is an error to investigate. Installers preserve unrelated same-name units and back up replaced owned units. No existing Compose service or unrelated Tailscale listener is replaced.

Configure a DHCP reservation for the LAN address, or rerun LAN generation/installation and restart the socket/service when the address changes. The publisher follows address changes, but systemd's explicit socket listeners require regeneration. When updating an active listener, restart `genie-local-proxy.socket` as well as its service. The optional `--mdns` step expects an existing nss-mdns resolver; omit it on systems whose resolver already supports the names.

Some Linux clients using `nss-mdns` minimal reject names with more than two labels, such as `dev.genie.local`. The local installer adds the full `mdns4` resolver with `/etc/mdns.allow` restricted to Genie suffixes and preserves the previous configuration in backup files. Other Linux clients may need the same resolver adjustment. Validate with `avahi-resolve -4 -n preview.genie.local` and `getent ahostsv4 preview.genie.local`; publishing mDNS does not guarantee every client's resolver configuration or Wi-Fi multicast support. See the [nss-mdns documentation](https://github.com/avahi/nss-mdns#activation).

mDNS is link-local and is not transported by Tailscale. For exactly the same HTTP names on remote devices, configure their `hosts` file or an appropriate private DNS server to map the three names to this host's Tailscale IP. On Linux, `files` must precede authoritative mDNS modules for `hosts` overrides to take effect. Fixed candidate hostnames also require explicit records when used remotely. The private HTTPS links printed by the helper work without these overrides.

Tailscale's **Add an app** form configures App Connectors, which discover routes to already resolvable applications; it does not create the desired mDNS names. Use the named Services below for stable private HTTPS addresses instead of overriding an unowned public domain.

## Named Tailscale Services

The selected remote entry points are `https://genie.<tailnet>.ts.net` for main, `https://genie-dev.<tailnet>.ts.net` for dev, and `https://genie-preview.<tailnet>.ts.net` for the latest integration candidate. The preview service also serves the auto-refreshing index at `/__previews`. Index links open fixed candidates, retaining the exact source composition during approval.

[Tailscale Services](https://tailscale.com/docs/features/tailscale-services) requires a tagged host identity. This installation uses a separate userspace `tailscaled` instance named `genie-service-host`, tagged `tag:genie-services`, with private state under the shared local helper directory and a socket at `$XDG_RUNTIME_DIR/genie-services/tailscaled.sock`. The existing user-owned machine and its T3 listener keep their identity, DNS settings and routes. The separate daemon sets `accept-dns=false` and `accept-routes=false` and advertises no subnet or exit-node routes.

The generated `genie-tailscale-services.service` and `genie-service-router.service` user units start the dedicated node and its loopback router. The router listens on 7491 (main), 7492 (dev) and 7493 (preview), resolves the current channel manifest on each request, and forwards through the candidate gateway. Each Service advertises `tcp:443`, proxies HTTPS to its matching loopback port, and needs an approved host advertisement. A new candidate does not need another Service or another approval of the host advertisement.

To provision another installation, first define the `genie-services` tag in **Access controls → Definitions → Tags** with the intended administrator as tag owner. Define `genie`, `genie-dev` and `genie-preview` in **Services**, each with endpoint `tcp:443`. Generate/install the dedicated node and router units (include `--lan` too when regenerating both sets), then start them:

```bash
python3 scripts/preview/install.py --services --install-user-units
systemctl --user enable --now genie-tailscale-services.service genie-service-router.service
```

Authenticate only the separate socket with the requested tag, then configure its endpoints:

```bash
service_socket="$XDG_RUNTIME_DIR/genie-services/tailscaled.sock"
tailscale --socket="$service_socket" login --hostname=genie-service-host \
  --advertise-tags=tag:genie-services --accept-dns=false --accept-routes=false
tailscale --socket="$service_socket" serve --service=svc:genie --https=443 http://127.0.0.1:7491
tailscale --socket="$service_socket" serve --service=svc:genie-dev --https=443 http://127.0.0.1:7492
tailscale --socket="$service_socket" serve --service=svc:genie-preview --https=443 http://127.0.0.1:7493
```

Approve that host on each Service's page and allow the intended clients to reach these Services under the tailnet's existing access policy. Preserve unrelated policy rules. Tailscale provisions HTTPS certificates; wait for actual successful HTTPS responses, not only an Online badge. Clients running Tailscale 1.94+ discover Service routes without enabling general subnet-route acceptance; consult the Tailscale guide for older clients.

```bash
preview enable-services
```

This verifies the dedicated node's identity/tag, discovers and saves its tailnet domain for the router, then checks all three application health responses against the expected channel candidate IDs before selecting the named preview index as the helper's default remote URL. Fixed candidate URLs still use their existing dedicated HTTPS ports on the original machine. Keep these two roles distinct: a stable channel name can advance; the fixed candidate identifies what the owner approved.

Existing LAN mDNS aliases remain available for devices without Tailscale. The named `ts.net` addresses work on devices connected to the tailnet, including while on the same LAN. After renaming the tailnet, rerun both enable commands and verify the real HTTPS URLs; no override of a third party's public domain is needed.

## Temporary public access

[Tailscale Funnel](https://tailscale.com/docs/features/tailscale-funnel) can give people without Tailscale a public HTTPS URL in the tailnet's `ts.net` domain. It is separate from private Serve access and does not make `.local` names public. Supported public ports are 443, 8443 and 10000. Preserve existing private listeners; reserve 10000 for an explicitly requested public share of one fixed candidate, never the environment index or live tracker.

Public sharing is not a worktree creation or settle action. Obtain the owner's requested candidate and expiry before enabling it. Keep application authentication enabled and provide access only to disposable preview data. A time limit must be enforced by a local gateway and cleanup timer; Funnel itself is not an expiring invitation. Public-share activation is not part of the installed automatic Actions, and no Funnel is enabled by this workflow.

## Owner approval and lifecycle

Before handing off, verify the application URL and its relevant behavior, then record candidate ID, base SHA, all task SHAs, local/remote URLs, check results, limitations and a short review scenario in the task. For a documentation-only task, provide the rendered document or the exact committed diff alongside the preview. Record `awaiting_visual_review` as a progress comment; keep the existing tracker status until a permitted workflow transition applies.

The owner's explicit response must identify the reviewed candidate and authorize publication. Local readiness and T3 thread state are not approval. Changed task commits, base or task composition need a new candidate and approval before pushing. An approval of A+B does not establish that A alone works: if a task is removed, review the new composition, and disclose genuine task dependencies in the PRs.

After approval, publish each task's approved branch to `dev`, link its PR and follow the existing CI/review workflow. Visual approval is separate from owner task acceptance and merge authorization.

```bash
preview stop <candidate-id>
preview restart <candidate-id>
```

Stopping terminates only recorded preview processes after checking their process identity. Restart reuses the same binary, web assets, port and disposable data. Candidate ports and Tailscale routes stay reserved for their original candidate so old links are never silently repointed. Stop unused candidates to release memory. Removing retained artifacts, git worktrees or reserved Serve mappings is an explicit local maintenance operation; the helper never deletes another task's worktree or database.

Local logs and diagnostic manifests live under the shared `genie-preview` directory. A successful health check confirms startup, not correctness of the task's acceptance criteria. Run the repository's applicable checks and manually verify the intended review scenario before requesting approval.
