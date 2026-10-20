# Coding standards

These rules apply to new and changed code. Follow nearby conventions for details and keep unrelated cleanup outside the task. Workflow and delivery requirements live in [AGENTS.md](AGENTS.md).

## Architecture and domain rules

- Keep `crates/genie-core` synchronous: domain types, tracker rules, SQLite persistence, migrations, and the event journal. Keep Tokio, HTTP, runtime orchestration, and channels in `crates/genie`.
- Route user, agent, and automation task commands through the shared application services in `crates/genie/src/tasks.rs`. Keep authorization, autonomy, and delivery gates there rather than reimplementing them in each transport. Follow the module's documented exception for internal bookkeeping within tracker transactions.
- Extend the operation catalog in `crates/genie/src/ops` for CLI/MCP operations. Preserve the shared request contract instead of creating independent versions for each client.
- Reuse domain enums and existing error types. Preserve distinctions between invalid input, forbidden actions, conflicts, missing records, and internal failures; make failures actionable to the caller.
- Consult [architecture](docs/platform/backend.md) and [decisions](docs/platform/decisions.md) when changing these boundaries. Update the relevant decision or contract documentation when the behavior changes.

## Rust and persistence

- Use the workspace toolchain, edition, and `rustfmt.toml`; format with `cargo fmt`. Keep Clippy clean with warnings treated as errors.
- Use the existing `App` blocking/database helpers when synchronous database work is called from async code. Keep blocking I/O off Tokio workers and release database locks before awaiting external work.
- Propagate recoverable errors through `Result`. Reserve panics and `expect` for established invariants or test setup, with enough context to identify the violated assumption.
- Keep persistent state changes and their journal events in the same transaction. Preserve idempotency and cursor behavior in retryable background work.
- Parameterize SQL values. Evolve existing databases through the versioned migration mechanisms; cover upgrade behavior, integrity, and rejection of schemas newer than the binary understands. Use temporary test databases instead of live tracker data.

## API contracts and TypeScript

- Define shared wire types in Rust and regenerate `web/src/shared/api/generated` through the Rust tests. Commit regenerated output with the contract change; edit its source rather than generated files.
- Keep strict TypeScript checking enabled. Prefer existing domain types, explicit unions, and narrowing over `any` or assertions that hide an unchecked boundary.
- Follow local import conventions: direct Node-executed tests and pi extensions use explicit `.ts` imports and Node-compatible TypeScript. Keep browser-only dependencies out of these modules.
- Preserve server-side authorization even when the web hides an action. Handle unknown or newer wire variants deliberately where compatibility requires it.

## Web UI

- Place code in the existing `app`, `pages`, `widgets`, `features`, `entities`, and `shared` layers according to its responsibility. Keep reusable lower layers independent of higher layers.
- Keep work pages and settings pages independent. Use shared modules for common behavior; follow `steiger.config.ts` for the boundaries and existing exceptions enforced today.
- Reuse `shared/api` request/error handling and TanStack Query conventions. Account for query invalidation and live events whenever a mutation changes visible state.
- Separate pure interaction/state logic from rendering when it permits meaningful behavioral tests, as existing feature `model.ts` modules do. Reuse shared UI primitives and nearby CSS conventions.
- Preserve the surrounding product language and user-facing terminology. Use semantic controls and labels, with keyboard interaction for new controls.

## Verification and documentation

- Add behavioral coverage for changed domain rules, authorization, state transitions, API contracts, and regression fixes. Test observable outcomes, including relevant error paths. Keep tests independent of live providers and production data unless an explicitly configured integration test requires them.
- Use the existing Rust unit/integration suites and Node `node:test` suites in `test/`. Avoid adding a test framework for cases these suites cover.
- Cover rendering, interaction, and layout of the web UI with the Playwright browser tests in `test/ui/` when a change alters what a screen shows or what a control does; keep pure UI logic in `node:test`. Update screenshot baselines only for intended visual changes. See [browser tests](docs/development.md#browser-tests).
- Run the checks relevant to the diff. The authoritative list and path filters are in [.github/workflows/ci.yml](.github/workflows/ci.yml); commands are also in [README.md](README.md#development) and `package.json`. Local success supports, but does not replace, successful PR CI.
- Explain intent, invariants, and surprising constraints in comments; keep names aligned with the domain. Use English for code identifiers and technical comments, preserving established user-facing text. Write project documentation in concise English.
- Update documentation and configuration examples when changing supported behavior. Keep secrets, local databases, generated build output, and machine-specific paths out of commits.
