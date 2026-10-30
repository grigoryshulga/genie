---
title: Development
summary: Build Genie, verify changes, and maintain the documentation website.
type: guide
status: current
---

# Development

Requires stable Rust, Node 22.19+, Git, and bubblewrap for Linux sandbox tests. See [Architecture](platform/backend.md) for the code map.

The [agent guide](agents/guide.md) defines how a task is tracked, isolated, published and cleaned up; `AGENTS.md` in the repository only points to it. In short:

- Iterate on the [development stand](development/testing-workflow.md): hot reload on varied seeded data.
- Run the [checks](#checks) that cover the changed paths; CI runs the same commands.
- A UI change is also shown to the owner with the [instant fixture preview](development/visual-preview.md) (`npm run preview:ui`, or a fixed integration candidate on request) and needs their approval before its PR is published. Every other change is published once its checks pass.
- The local tracker follows `dev` when you run `preview update-tracker` (see the [tracker updater](development/tracker-auto-update.md)), and `preview gc` frees disk space ([testing workflow](development/testing-workflow.md#keeping-the-disk-clean)).

The tracker task commands are in the [genie CLI guide](development/genie-cli.md).

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

For web development, use the stand: it builds `genie`, seeds a disposable data directory and starts Vite's hot-reload server in front of it, with no image build or restart:

```bash
npm run dev:stand
```

To run Vite alone against a server you already have, use `npm run dev:web`. It proxies `/api` to `http://127.0.0.1:7420` unless `GENIE_API_PROXY_TARGET` selects another server, for example the primary tracker at `http://127.0.0.1:7422`; actions then change that server's real tasks.

For production-image validation, the Docker web stage builds and compresses the static assets independently from the Rust binary, so after the first image build a web-only rebuild reuses the cached Rust stage. Build the image without deploying it:

```bash
docker compose -p genie-tracker -f docker/docker-compose.tracker.yml build
```

The running tracker is changed only by `preview update-tracker` or the optional timer, never by a build from an unmerged checkout.

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

`test/ui/run.ts` seeds two disposable data directories through the CLI (`test/ui/seed.ts`), starts one server on each, and runs Playwright inside `mcr.microsoft.com/playwright` at the version of the installed `@playwright/test`. The screenshots use one server and the flows the other, so flows never change the screenshot data. Running Playwright only in that image keeps fonts and rendering identical locally and in CI. Set `GENIE_BIN` to test another server binary; arguments after `--` go to Playwright, for example `npm run test:ui -- --project flows`.

- `*.flow.ts` files are user flows. Assert what the user sees and, through `test/ui/api.ts`, what the server stored.
- `*.visual.ts` files take screenshots at desktop and mobile sizes. The page clock is fixed one hour after seeding, so relative times stay stable.
- After an intended visual change, run `npm run test:ui -- --update-snapshots` and review the changed images in `test/ui/__screenshots__/` before committing them.

### Accessibility

`test/ui/a11y.ts` is a small rule engine that runs inside a rendered page; `test/ui/a11y.visual.ts`
runs it over every screen the standard seed reaches, in both themes, and fails on anything critical
or serious. It also walks the keyboard road on `/tasks` and `/board` and fails on a stop with no
visible focus, checks that nothing still animates under `prefers-reduced-motion`, and checks that the
phone filter sheet keeps focus while it is open and gives it back when it closes.
`test/ui/a11y.flow.ts` finishes the jobs themselves with the keyboard alone — create a task, open a
task, move a card, open a document, answer an owner decision — and reads the server afterwards; these
tests never click. `test/accessibility.test.ts` and `test/design-tokens.test.ts` cover what needs no
browser: the focus hook, the decision box's hand-off, and contrast pairs for every colour token.

The rules are the promises the design system makes, not a conformance audit: named controls, sound
roles, contrast, target size, headings, `image-alt`, duplicate ids, `aria-hidden` focus, nested
interactive elements, positive `tabindex`, a main landmark, a named modal, and SVG names.

What it does not check: axe or any third-party auditor, full WCAG 2.2 A/AA coverage, screen readers
or real assistive technology, zoom and reflow at 400 %, text spacing, motion sickness beyond
`prefers-reduced-motion`, language of parts, tables, live-region announcement ordering, or the
screens only the capture seed reaches (`npm run ui:capture`). The moderate findings that remain are
printed, not fatal; the current ones are touch targets under 24×24 px on the section collapse toggle,
the task-list title link and a few inline links, in both themes.

### Screenshot capture

`npm run ui:capture` photographs every route in `test/ui/routes.ts` and compares nothing. It uses the same server, seed and image as the browser tests, plus extra tasks (`CAPTURE_TASKS` in `test/ui/seed.ts`) so that all eleven statuses, an owner question and an epic with children are present. The seed never touches a real tracker or data directory.

```bash
npm run ui:capture                                  # the application as it renders
GENIE_UI_THEMES=ink,paper npm run ui:capture        # one set per data-theme value
```

PNGs land in `test-results/captures/<route>-<theme>-<1440|390>.png` (ignored by git), at 1440x900 and 390x844, with animations disabled and the clock fixed. Attach the before and after sets to the UI task that changed them. Known gaps: team and agent chat screens need running agents, so they are not captured; full-page shots do not include content scrolled inside a panel (the board's wide lanes are clipped to the viewport); the server health page shows the throwaway data path.

Reports, traces, and screenshot diffs are written to `test-results/`; CI uploads them when the `Browser tests` job fails. CI runs that job (`.github/workflows/browser-tests.yml`) only on pull requests into `main` (releases), because it is slow, and every night on `dev`; it can also be started by hand from the Actions tab. Run `npm run test:ui` locally before a change that touches screens. Open a trace with `npx playwright show-trace <path>/trace.zip`.

The [agent benchmark](https://github.com/grigoryshulga/genie/tree/main/bench) compares task outcomes before and after changes to roles, prompts, skills, or team templates.

## Documentation site

The site uses VitePress with a separate package and lockfile in `site/`. Application dependencies do not include the documentation toolchain. The package overrides Vite to a patched 6.x release while retaining stable VitePress 1.x.

The pages are not stored in this repository. The project's documentation lives in the private knowledge vault (project `genie`, space `genie/`). `site/scripts/assemble.mjs` copies that space into `site/content/`, which is generated and ignored by git, together with the brand sources in `design-system/brand/`, the static files in `site/public/` and a generated `changelog.md` page that includes `CHANGELOG.md`.

```bash
npm ci --prefix site
npm run docs:vault    # clone or update the vault at site/.vault over SSH
npm run docs:dev      # http://127.0.0.1:5173/genie/; assembles content first
npm run docs:check    # English content and local Markdown links; assembles content first
npm run docs:build    # static site and search index; assembles content first
npm run docs:preview  # production build at http://127.0.0.1:4173/genie/
```

Set `GENIE_VAULT_DIR` to build from another checkout that contains the `genie/` space. Edit pages in the vault, never in `site/content/`; `npm run docs:assemble` regenerates that directory. The site navigation is in `site/.vitepress/config.mts`; styles and the landing page are in `site/.vitepress/theme/`. `CHANGELOG.md` at the repository root is still the source of the Changelog page, so release notes have one home.

Write in English. Start with the action or contract, keep examples runnable, and link to an existing explanation instead of repeating it. Put proposals under Roadmap and mark historical pages deprecated. Retain a short pointer at an old page path when replacing it.

Use relative Markdown links with `.md` extensions between vault pages so they work in the vault and on the site. Link to files in this repository with absolute GitHub URLs, because a relative path from a vault page cannot resolve. Keep wiki-link syntax inside code examples when documenting the vault contract. The build rejects dead page links; the content check also verifies local link fragments and rejects Cyrillic in documentation. The glossary, which maps the product's Russian UI labels, is the one page the check allows to keep them.

## GitHub Pages

`.github/workflows/docs.yml` checks and builds the site for pull requests and pushes affecting the site or its sources. It checks out the vault into `site/.vault` with the read-only deploy key secret `GENIE_VAULT_DEPLOY_KEY`. Because a content change lands in the vault rather than here, the workflow also runs daily and on a `vault-updated` repository dispatch. Pushes to `main` deploy the built artifact to GitHub Pages; pull requests only validate. The workflow can also be run manually.

In the repository's **Settings → Pages**, set **Source → GitHub Actions**. After the workflow is merged into `main`, its successful deployment serves [grigoryshulga.github.io/genie](https://grigoryshulga.github.io/genie/).

The configured base path is `/genie/`. Update `base`, the canonical URL, and favicon path in the VitePress config if moving the site to a custom domain or a different repository. The `github-pages` environment may require approval if repository protection rules demand it.
