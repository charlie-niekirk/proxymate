# Proxymate

A fast desktop HTTP(S) debugging proxy (in the vein of Proxyman / Charles), built with Tauri 2, a Rust core and a React + TypeScript UI. macOS first, then Linux and Windows.

**`docs/PLAN.md` is the source of truth** for architecture, scope and milestones. Read it before starting work. If your work changes the plan, update `docs/PLAN.md` in the same PR.

## Delivery workflow (mandatory)

- Each milestone (M0–M12 in `docs/PLAN.md`) is built in **its own Claude session** and lands as **its own PR**.
- Branch from the latest `main` as `mN-<slug>` (for example `m1-http-proxy`). Use the branch name the Milestones table gives.
- Do not implement more than one milestone in a session, and do not start the next one.
- Open the PR titled `MN: <Title>`. Its description covers:
  - what shipped;
  - how it was verified, with commands and screenshots;
  - any deviations from `docs/PLAN.md`;
  - follow-ups.
- **Never merge your own PR.** `main` is protected: changes land only through PRs with green CI.
- Done means:
  - CI is green;
  - the milestone's verify column passes;
  - the PR is open for review.

## Commands

Run these from the repo root unless noted.

| What | Command |
|---|---|
| Install JS deps | `npm install` (npm workspaces; pnpm is not used) |
| Run the app | `npm run tauri dev` |
| Run the app with the perf probe | `PROXYMATE_SYNTH_FLOWS=100000 VITE_PM_PERF_PROBE=1 npm run tauri dev`. Scroll and filter timings print in the terminal as `[perf-probe]` |
| Run the app with Auto Select on | `VITE_PM_AUTO_SELECT=1 npm run tauri dev`. New traffic is selected and inspected without a click (for verifying from a shell) |
| Headless proxy | `cargo run -p proxymate-cli -- serve [PORT]`. Logs each finished flow |
| Test upstream by hand | `cargo run -p pm-testkit --bin upstream`, then `curl -x http://127.0.0.1:9090 http://127.0.0.1:<port>/json` |
| TLS test upstream (h2 + h1) | `cargo run -p pm-testkit --bin upstream -- --tls` prints its port and CA file. Start the app with `PROXYMATE_EXTRA_UPSTREAM_CA=<that file>`, then `curl -x http://127.0.0.1:9090 --cacert ~/Library/Application\ Support/Proxymate/ca/proxymate-ca.pem https://localhost:<port>/json --http2 -v` |
| Open the app on a response tab | `VITE_PM_RESPONSE_TAB=TLS` (or `Timing`, …), for screenshots from a shell |
| Drive the app from a shell | Start with `VITE_PM_REMOTE=1`, then `curl -d '{"select":3,"requestTab":"Body"}' localhost:1420/__pm_remote` (also `responseTab`, `viewer: {kind, viewer}`, `theme`). See `src/dev/remote.ts` |
| Body-viewer zoo through the proxy | `cargo run -p pm-testkit --bin upstream -- --zoo [127.0.0.1:9090]` sends one flow per fixture in `crates/pm-testkit/fixtures/zoo/` |
| Zoo in the browser UI | `npm run dev`, then open `http://localhost:1420/?zoo` (mock backend serving the zoo; `&select=<id>&theme=light&requestTab=Body`) |
| Viewer snapshots + preview safety (Playwright) | `npm run e2e` (first time: `npx -w @proxymate/ui playwright install webkit chromium`). `-- --update-snapshots` after an intended visual change; `PM_E2E_PORT` picks the dev server port (default 1425) |
| UI only, in a browser | `npm run dev` (http://localhost:1420, in-memory mock backend) |
| Rust format / lint / test | `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` |
| UI typecheck / lint / test / build | `npm run typecheck`, `npm run lint`, `npm run test`, `npm run build` |
| Regenerate TS bindings | `cargo test -p proxymate-desktop export_bindings`. Debug app launches also regenerate them |
| Engine micro-bench | `cargo run --release -p proxymate-cli -- bench [FLOWS]` |
| Ingest bench (criterion, target < 2 s per 100k flows) | `cargo bench -p pm-core --bench ingest` |
| Release bundle | `npm run tauri build` |

CI (`.github/workflows/ci.yml`):
- Rust: `fmt --check`, `clippy -D warnings`, `test` and a bindings-drift check, on macOS and Linux.
- UI: `typecheck`, `lint`, `vitest`, `build` and Playwright (`e2e`, pixel snapshots off on Linux: baselines are macOS-only for now).

Run the same checks locally before pushing.

## Layout and crate boundaries

```
crates/pm-model      pure types (serde + specta): FlowSummary, FlowRow, FilterSpec, SortSpec, ViewDelta, ...
crates/pm-core       headless Engine: proxy/ (h1 forward proxy, mitm.rs: CONNECT, MITM, h2, passthrough), tls/ (CA, leaf
                     cache, SSL policy, upstream verifier), recorder (per-flow capture), ingest (event thread),
                     store/ (arena + views, segments, bodies, details), io/ (content kind, decoding), synth; later rules/
crates/pm-platform   OS integration traits + cfg impls (fs: private files; sysproxy, trust, procinfo, ... from M4)
crates/pm-mcp        MCP server over Arc<Engine>, empty until M7
crates/pm-testkit    test upstreams (h1, TLS h1/h2 with a test CA), the body-viewer zoo (fixtures/zoo), proxy and HTTPS clients;
                     re-export of pm_core::synth
apps/desktop/src-tauri   thin Tauri adapters (commands.rs), the pmbody:// scheme (body_scheme.rs), window chrome (window.rs)
apps/desktop/ui          Vite + React 19 + TS strict + Tailwind v4 + shadcn/ui
apps/cli                 proxymate-cli (headless; `bench` today)
```

Rules:
- **Adapters call only the `Engine` facade** (`pm_core::Engine`). Tauri commands, MCP tools and the CLI never reach into `pm_core::store` or other internals. This keeps the UI and agents behaving the same. If an adapter needs something new, add it to `Engine`.
- `pm-core` has **no Tauri dependency** and stays headless and testable.
- `pm-model` holds data types only: no I/O and no engine logic. Every type that crosses IPC lives here and derives `Serialize`, `Deserialize` and `specta::Type`.
- OS-specific code goes in `pm-platform` behind traits, never `cfg`-sprinkled through `pm-core`.

## Data path (performance core)

1. Traffic handlers record through a `FlowRecorder` (`Engine::begin_flow`). Summary changes go as small `FlowEvent`s over one channel to the `pm-ingest` thread, which applies each batch (up to 5 ms) under one write lock. Views update incrementally. Bodies never travel in events: they are tee'd into `store/blob.rs` (inline up to 8 KB, else segment files, capped at 64 MB, still content-encoded).
2. The publisher thread ticks at 20 Hz and sends one `ViewDelta` per subscribed view over a `tauri::ipc::Channel`. A delta carries `{total, len, appended, changed ids, structural}`.
3. The UI (`hooks/useFlowView.ts` and `lib/rowCache.ts`) fetches only the visible window plus one page of read-ahead with `get_rows(view, start, count)`. Rows arrive as positional `FlowRow` tuples, decoded by `lib/flow.ts`.
4. `set_filter` and `set_sort` refilter synchronously in Rust (rayon above 50k flows) and return the new length right away.

Targets:
- 100k rows scroll at 60 fps.
- A filter change repaints in under 50 ms end to end.
- An engine refilter takes under 10 ms.
- `FlowRow` field order is part of the IPC contract. If you change it, update `decodeRow` and its test.

## Conventions

**Rust**
- Edition 2024, with `rustfmt.toml` at `max_width = 120`.
- Clippy must pass with `-D warnings`. `unsafe_code` is denied workspace-wide; scope any exception with `#[allow]` and a justification.
- Errors use `thiserror` in library crates. Tauri commands map errors to `String` for now.
- Each module has a short `//!` doc. Comments explain *why*, not *what*.

**TypeScript**
- `strict` and `noUncheckedIndexedAccess` are on. Import with the `@/` alias.
- Use the generated `src/bindings.ts` types. Never edit that file by hand, never hand-write IPC types, and call commands through `lib/backend.ts`.

**State**
- Zustand (`store/ui.ts`) holds UI state.
- Row data lives in the non-reactive `RowCache`, never in React state.
- TanStack Query is planned for detail, bodies and rules.

**Design system: shadcn/ui** (Radix + Tailwind v4)
- The components are copied into `src/components/ui/` and restyled to a compact density: 12–13 px text and 24–28 px controls. Add more with `npx shadcn@latest add <name>` from `apps/desktop/ui`.
- Check the result: once the CLI rewrote the `cn` import to a stray npm package.

**Colours**
- Every colour is a token in `src/styles/tokens.css`: shadcn variables plus `--m-*` (methods), `--s-*` (status and state), `--url-*` and `--selection-*`. Don't hard-code colours in components.
- Dark is the primary theme; light gets a full pass in M10.

**Body viewers** (`src/components/BodyViewer/`)
- `registry.ts` maps each `ContentKind` to its viewers, best first. Add a viewer there and in `Viewer.tsx`.
- Media, images, PDFs and fonts load from `backend.bodyUrl` (`pmbody://` in the app, blob URLs in the mock); text viewers get decoded bytes over IPC.
- Big work runs in workers: JSON parsing/formatting above 256 KB, and prettier (lazy-loaded) for Beautify.
- The HTML preview must stay sandboxed (no `allow-scripts`), URL-stripped and under `PREVIEW_CSP` (`lib/body/htmlPreview.ts`); `e2e/security.spec.ts` checks it.

**The flow table**
- It is a custom div grid virtualised with TanStack Virtual, at a fixed 22 px row height. Do not swap in shadcn's DataTable.
- Keep row components memoised, with stable callbacks.

**Window chrome**
- The overlay title bar with inset traffic lights is set in `tauri.conf.json`.
- The window is transparent and `window.rs` applies sidebar vibrancy. Panes must paint opaque backgrounds, except the sidebar (`--sidebar` is transparent on macOS).

**Proxy and synthetic data**
- The app starts the proxy on 127.0.0.1:9090, falling back to 9091–9099. The toolbar shows the bound port, or why it couldn't bind.
- Synthetic data is opt-in: `PROXYMATE_SYNTH_FLOWS` seeds flows and `PROXYMATE_SYNTH_RATE` adds that many live ones per second. Both default to 0. Keep `synth` for tests and benches.
- Live segment files live under `~/Library/Application Support/Proxymate/live/<uuid>/`. Clear and quit delete them; the next launch sweeps any left by a crash.
- HTTPS: the CA is `<data dir>/ca/proxymate-ca.pem` (key `proxymate-ca-key.pem`, mode 0600) and the SSL lists are `<data dir>/ssl.json`. Upstreams are verified with the OS trust store plus `EngineConfig::extra_upstream_roots` (`PROXYMATE_EXTRA_UPSTREAM_CA` in the app and CLI). Give each test engine its own `data_dir` so tests don't share a CA.
- Integration tests are in `crates/pm-core/tests/`, using `pm-testkit` upstreams and clients. Tests that need their own process (RSS, environment variables) get their own file.
