---
title: Development
summary: Build Genie, verify changes, and maintain the documentation website.
type: guide
status: current
---

# Development

Requires stable Rust, Node 22.19+, Git, and bubblewrap for Linux sandbox tests. See [Architecture](platform/backend.md) for the code map.

Follow the [repository workflow](development/genie-cli.md) to track a task, isolate changes, and publish a PR with successful CI.

Before publishing a task branch or opening its PR, prepare a [fixed integration preview](development/visual-preview.md) and obtain the owner's visual approval. That guide also documents machine-local T3 Project Actions, combined task candidates and private Tailscale access.

## Release flow

Feature work enters the protected `dev` branch through pull requests. Releases go from `dev` to the protected `main` branch through a pull request; direct pushes and force pushes to either branch are blocked.

After a pull request is merged into `dev`, GitHub Actions deletes its source branch when that branch belongs to this repository. The protected `dev` and `main` branches are retained.

Before opening a release PR to `main`, increase the shared version in `Cargo.toml` and `package.json`, update `Cargo.lock` and `package-lock.json`, and add a dated section for that version in `CHANGELOG.md`. The required release gate checks that the PR comes from `dev` and that the version files and changelog agree. When the PR is merged, GitHub Actions creates the `v<version>` tag and published GitHub Release using those notes. Docker images are built and published on pushes to `main` only.

## Application

```bash
npm ci
npm run build:web
GENIE_WEB_DIST=web/dist cargo build -p genie
cargo run -p genie -- serve
```

For web development, run Vite's HMR server without rebuilding or restarting Genie:

```bash
npm run dev:web
```

By default Vite proxies `/api` to `http://127.0.0.1:7420`. To work against the primary local tracker instead, set its target:

```bash
GENIE_API_PROXY_TARGET=http://127.0.0.1:7422 npm run dev:web
```

The UI updates immediately while requests use the selected Genie server. For production-image validation, the Docker web stage builds and compresses the static assets independently from the Rust binary. After the first image build, a web-only Docker rebuild reuses the cached Rust stage. To rebuild and recreate the local tracker with its explicit project and Compose file:

```bash
docker compose -p genie-tracker -f docker/docker-compose.tracker.yml up -d --build
curl -fsS http://127.0.0.1:7422/api/health
```

`genie serve --web web/dist` also serves a local static build without rebuilding the binary.

## Checks

Live-session Rust tests use `pi` from `PATH`. CI installs the same Pi-Bolt version and SHA-256
as `docker/Dockerfile`; Node is still needed for the fake stdio MCP servers. On Linux x86_64,
install that build locally without replacing your system's `pi`:

```bash
bash .github/scripts/install-pi-bolt.sh "$PWD/target/pi-bolt"
export PATH="$PWD/target/pi-bolt:$PATH"
pi --version
```

The live-session tests skip locally when `pi` is absent and fail on CI if it is missing.

With a warm build, `cargo nextest run --workspace --lib --bins --tests --test-threads 8` runs
all Rust test binaries concurrently; CI uses nextest 0.9.146 to report individual test durations.
`cargo test` remains available. Timer-sensitive tests
use a per-application manual clock and completed scheduler/reconciliation passes; real Pi-Bolt,
MCP, Git and sandbox checks remain in the full suite. Keep subprocess and HTTP I/O timeouts real:
automatically advancing all Tokio timers can turn ordinary I/O into artificial timeouts.

For a shorter unit/domain feedback loop, run `cargo test --workspace --lib`. Run the full
suite below before publishing; the short command excludes the HTTP/process integration binaries.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
npm test
npm run typecheck
npm run lint:web
npm run build:web
```

Rust tests regenerate `web/src/shared/api/generated/`. Commit generated type changes with their source structs. Set `GENIE_WEB_SOURCEMAP=1` when building the web client for browser debugging; source maps are not embedded in the binary.

### Browser tests

The browser tests in `test/ui/` click through the web UI and compare screenshots of key screens with committed baselines. They need Docker, a built server, and a built web client:

```bash
cargo build -p genie
npm run build:web
npm run test:ui
```

`test/ui/run.ts` seeds a disposable data directory through the CLI (`test/ui/seed.ts`), starts two servers from copies of it, and runs Playwright inside `mcr.microsoft.com/playwright` at the version of the installed `@playwright/test`. The screenshots use one server and the flows the other, so flows never change the screenshot data. Running Playwright only in that image keeps fonts and rendering identical locally and in CI. Set `GENIE_BIN` to test another server binary; arguments after `--` go to Playwright, for example `npm run test:ui -- --project flows`.

- `*.flow.ts` files are user flows. Assert what the user sees and, through `test/ui/api.ts`, what the server stored.
- `*.visual.ts` files take screenshots at desktop and mobile sizes. The page clock is fixed one hour after seeding, so relative times stay stable.
- After an intended visual change, run `npm run test:ui -- --update-snapshots` and review the changed images in `test/ui/__screenshots__/` before committing them.

### Screenshot capture

`npm run ui:capture` photographs every route in `test/ui/routes.ts` and compares nothing. It uses the same server, seed and image as the browser tests, plus extra tasks (`CAPTURE_TASKS` in `test/ui/seed.ts`) so that all eleven statuses, an owner question and an epic with children are present. The seed never touches a real tracker or data directory.

```bash
npm run ui:capture                                  # the application as it renders
GENIE_UI_THEMES=ink,paper npm run ui:capture        # one set per data-theme value
```

PNGs land in `test-results/captures/<route>-<theme>-<1440|390>.png` (ignored by git), at 1440x900 and 390x844, with animations disabled and the clock fixed. Attach the before and after sets to the UI task that changed them. Known gaps: team and agent chat screens need running agents, so they are not captured; full-page shots do not include content scrolled inside a panel (the board's wide lanes are clipped to the viewport); the server health page shows the throwaway data path.

Reports, traces, and screenshot diffs are written to `test-results/`; CI uploads them when the `Browser tests` job fails. Open a trace with `npx playwright show-trace <path>/trace.zip`.

The [agent benchmark](https://github.com/grigoryshulga/genie/tree/main/bench) compares task outcomes before and after changes to roles, prompts, skills, or team templates.

## Documentation site

The site uses VitePress with a separate package and lockfile in `docs/`. Application dependencies do not include the documentation toolchain. The package overrides Vite to a patched 6.x release while retaining stable VitePress 1.x.

```bash
npm ci --prefix docs
npm run docs:dev      # http://127.0.0.1:5173/genie/
npm run docs:check    # English content and local Markdown links
npm run docs:build    # static site and search index
npm run docs:preview  # production build at http://127.0.0.1:4173/genie/
```

Edit the Markdown in `docs/`. The site navigation is in `docs/.vitepress/config.mts`; styles and the landing page are in `docs/.vitepress/theme/`. `CHANGELOG.md` at the repository root is included in the site, so release notes have one source.

Write in English. Start with the action or contract, keep examples runnable, and link to an existing explanation instead of repeating it. Put proposals under Roadmap and mark historical pages deprecated. Retain a short pointer at an old page path when replacing it.

Use relative Markdown links with `.md` extensions so they work in GitHub and on the site. Keep wiki-link syntax inside code examples when documenting the vault contract. The build rejects dead page links; the content check also verifies local link fragments and rejects Cyrillic in documentation.

## GitHub Pages

`.github/workflows/docs.yml` checks and builds the site for pull requests and pushes affecting documentation. Pushes to `main` deploy the built artifact to GitHub Pages; pull requests only validate. The workflow can also be run manually.

In the repository's **Settings → Pages**, set **Source → GitHub Actions**. After the workflow is merged into `main`, its successful deployment serves [grigoryshulga.github.io/genie](https://grigoryshulga.github.io/genie/).

The configured base path is `/genie/`. Update `base`, the canonical URL, and favicon path in the VitePress config if moving the site to a custom domain or a different repository. The `github-pages` environment may require approval if repository protection rules demand it.
