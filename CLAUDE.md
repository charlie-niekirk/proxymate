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
| Headless proxy | `cargo run -p proxymate-cli -- serve [PORT]`. Logs each finished flow. `--mcp[=PORT]` also serves MCP (prints the `claude mcp add` line) |
| MCP Inspector against the app | `npx @modelcontextprotocol/inspector --cli http://127.0.0.1:9095/mcp --transport http --header "Authorization: Bearer $(sqlite3 ~/Library/Application\ Support/Proxymate/app.sqlite "select json from settings where key='mcp_token'" \| tr -d '"')" --method tools/list` (add `--strict` for schema portability) |
| Stdio MCP bridge | `proxymate mcp-stdio` (app binary) or `cargo run -p proxymate-cli -- mcp-stdio [--port N] [--no-launch]` (token from `PROXYMATE_DATA_DIR`, else the app's data dir) |
| Test upstream by hand | `cargo run -p pm-testkit --bin upstream`, then `curl -x http://127.0.0.1:9090 http://127.0.0.1:<port>/json`. It also serves a WebSocket echo at `/ws` and a slow event stream at `/sse?count=N&interval_ms=M` |
| WebSocket client through the proxy | `cargo run -p pm-testkit --bin ws-client -- ws://127.0.0.1:<port>/ws` (CONNECT; `--absolute` for absolute form) or `wss://localhost:<port>/ws` against the TLS upstream. `--count`, `--interval-ms`, `--server-close` |
| TLS test upstream (h2 + h1) | `cargo run -p pm-testkit --bin upstream -- --tls` prints its port and CA file. Start the app with `PROXYMATE_EXTRA_UPSTREAM_CA=<that file>`, then `curl -x http://127.0.0.1:9090 --cacert ~/Library/Application\ Support/Proxymate/ca/proxymate-ca.pem https://localhost:<port>/json --http2 -v` |
| Open the app on a response tab | `VITE_PM_RESPONSE_TAB=TLS` (or `Timing`, …), for screenshots from a shell |
| Drive the app from a shell | Start with `VITE_PM_REMOTE=1`, then `curl -d '{"select":3,"requestTab":"Body"}' localhost:1420/__pm_remote` (also `responseTab`, `viewer: {kind, viewer}`, `theme`; M8: `replay`, `editReplay`, `compose`, `composeSend`, `store`, `openDocument`, `saveSession`, `exportHar`, and `curl`, whose answer is at `/__pm_remote/out`). See `src/dev/remote.ts` |
| Body-viewer zoo through the proxy | `cargo run -p pm-testkit --bin upstream -- --zoo [127.0.0.1:9090]` sends one flow per fixture in `crates/pm-testkit/fixtures/zoo/` |
| Zoo in the browser UI | `npm run dev`, then open `http://localhost:1420/?zoo` (mock backend serving the zoo; `&select=<id>&theme=light&requestTab=Body`) |
| Viewer snapshots + preview safety (Playwright) | `npm run e2e` (first time: `npx -w @proxymate/ui playwright install webkit chromium`). `-- --update-snapshots` after an intended visual change; `PM_E2E_PORT` picks the dev server port (default 1425) |
| Open the app with the cert wizard / a search | `VITE_PM_OPEN=cert-wizard` (or `settings:mcp`), `VITE_PM_FILTER=<text>` (dev only, for screenshots) |
| Open the app on device setup | `VITE_PM_OPEN=device-setup:iphone` (or `simulator`, `android`, `emulator`). With `VITE_PM_REMOTE=1`, `{"deviceTab":"android"}` switches tabs and `{"click":"<data-testid>"}` clicks a control (e.g. `simulator-install`, `lan-access`) |
| Allow prompt in the browser UI | `npm run dev`, then open `http://localhost:1420/?devicePrompt=192.168.1.42` (mock LAN client waiting to be allowed) |
| Run the app as the system proxy | `PROXYMATE_SYSTEM_PROXY=1 npm run tauri dev`. Changes the real macOS proxy settings: restored on quit, on signals, and by the guardian after `kill -9` |
| UI only, in a browser | `npm run dev` (http://localhost:1420, in-memory mock backend) |
| Rust format / lint / test | `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` |
| UI typecheck / lint / test / build | `npm run typecheck`, `npm run lint`, `npm run test`, `npm run build` |
| Regenerate TS bindings | `cargo test -p proxymate-desktop export_bindings`. Debug app launches also regenerate them |
| Engine micro-bench | `cargo run --release -p proxymate-cli -- bench [FLOWS]` |
| Soak (M10) | `cargo run --release -p proxymate-cli -- bench --rps 500 --duration 3600 [--max-flows N]` (own engine), or `--proxy 127.0.0.1:9090 --pid <app pid>` against the app. CSV every `--interval` seconds: rate, p50/p99, failures, RSS, flows, disk, evicted |
| Ingest bench (criterion, target < 2 s per 100k flows) | `cargo bench -p pm-core --bench ingest` |
| Release bundle | `npm run tauri build` |

CI (`.github/workflows/ci.yml`):
- Rust: `fmt --check`, `clippy -D warnings`, `test` and a bindings-drift check, on macOS and Linux.
- UI: `typecheck`, `lint`, `vitest`, `build` and Playwright (`e2e`), in the Playwright Docker image with pixel diffs on. Linux baselines (`*-linux.png`) are made there: a missing one fails the run and comes back in the `playwright-report` artifact to commit.

Run the same checks locally before pushing.

## Layout and crate boundaries

```
crates/pm-model      pure types (serde + specta): FlowSummary, FlowRow, FilterSpec, SortSpec, ViewDelta, ...
crates/pm-core       headless Engine: proxy/ (h1 forward proxy, mitm.rs: CONNECT, MITM, h2, passthrough, ws.rs: WebSocket
                     relay, send.rs: Engine::send), tls/ (CA, leaf cache, SSL policy, upstream verifier), recorder
                     (per-flow capture), ingest (event thread), store/ (arena + views, sidebar counts, backend.rs:
                     BlobBackend, segments, sqlite.rs: session tables + SqliteBackend, bodies, details, messages: WS/SSE
                     logs), io/ (content kind, decoding, sse parser, curl, har, session), documents (opened sessions and
                     HARs), replay, client (loopback peer → app), trust (handshake check), integration (the OS-facing
                     Engine methods), rules/, synth
crates/pm-platform   OS integration traits; macos/ impls, stub.rs elsewhere: fs (private files), sysproxy (backup +
                     restore over a ProxyBackend; networksetup), trust (keychain), procinfo (libproc port scan, app
                     bundles, icons), process (kqueue exit wait), netif (if-addrs), simctl and adb (over a
                     command::Runner, faked in tests)
crates/pm-mcp        MCP server over Arc<Engine> (rmcp, pinned): http.rs (axum endpoint, bearer token, managed start/stop),
                     handler.rs (sessions, read-only, audit, notifications), tools.rs, resources.rs, prompts.rs,
                     bridge.rs (mcp-stdio); tests/mcp.rs drives an rmcp client against a headless engine
crates/pm-testkit    test upstreams (h1, TLS h1/h2 with a test CA, WS echo, slow SSE), the body-viewer zoo (fixtures/zoo),
                     proxy, HTTPS and WebSocket clients; re-export of pm_core::synth
apps/desktop/src-tauri   thin Tauri adapters (commands.rs), the pmbody:// scheme (body_scheme.rs), window chrome (window.rs),
                         the menu bar (menu.rs: File items reach the UI as `pm-menu` events)
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
- Zustand (`store/ui.ts`) holds UI state; `store/prefs.ts` holds what persists across launches (appearance, columns, sort).
- Row data lives in the non-reactive `RowCache`, never in React state.
- TanStack Query is planned for detail, bodies and rules.

**Design system: shadcn/ui** (Radix + Tailwind v4)
- The components are copied into `src/components/ui/` and restyled to a compact density: 12–13 px text and 24–28 px controls. Add more with `npx shadcn@latest add <name>` from `apps/desktop/ui`.
- Check the result: once the CLI rewrote the `cn` import to a stray npm package.

**Colours and themes**
- Every colour is a token in `src/styles/tokens.css`: shadcn variables plus `--m-*` (methods), `--s-*` (status and state), `--url-*`, `--selection-*` and the rest. Don't hard-code colours in components (not even `bg-white` or `bg-black/50`: there are `--paper` and `--overlay`).
- Both themes are first-class. `src/styles/contrast.test.ts` checks 4.5:1 for every text token on its surface (pills composited over the table) in dark and light; add a pair there when you add a text token.
- Settings › Appearance (System/Light/Dark) lives in `store/prefs.ts` (local storage) with the column layout and sort; `lib/theme.ts` applies it and syncs the native window theme. `?theme=light` pins a theme in dev (Playwright).

**Body viewers** (`src/components/BodyViewer/`)
- `registry.ts` maps each `ContentKind` to its viewers, best first. Add a viewer there and in `Viewer.tsx`.
- Media, images, PDFs and fonts load from `backend.bodyUrl` (`pmbody://` in the app, blob URLs in the mock); text viewers get decoded bytes over IPC.
- Big work runs in workers: JSON parsing/formatting above 256 KB, and prettier (lazy-loaded) for Beautify.
- The HTML preview must stay sandboxed (no `allow-scripts`), URL-stripped and under `PREVIEW_CSP` (`lib/body/htmlPreview.ts`); `e2e/security.spec.ts` checks it.

**The flow table**
- It is a custom div grid virtualised with TanStack Virtual, at a fixed 22 px row height. Do not swap in shadcn's DataTable.
- Keep row components memoised, with stable callbacks.
- Columns are defined in `FlowTable/columns.ts`; a new one also needs a `RowCell` case and a place in `DEFAULT_LAYOUT` (saved layouts pick it up at its default position).

**Keyboard and commands (M10)**
- `lib/commands.ts` is the single list: the ⌘K palette shows every command, and a command's `key` binds ⌘/Ctrl+key in `hooks/useShortcuts.ts`. Add commands there, not ad-hoc `keydown` listeners. ⌘⌫ never fires in text fields; ⌘F inside CodeMirror is left to its search.
- Errors from actions go to `reportError` (a sonner toast); a failed flow's explanation and fixes come from `lib/errors.ts`.

**Window chrome**
- The overlay title bar with inset traffic lights is set in `tauri.conf.json`.
- The window is transparent and `window.rs` applies sidebar vibrancy. Panes must paint opaque backgrounds, except the sidebar (`--sidebar` is transparent on macOS).

**Proxy and synthetic data**
- The app starts the proxy on 127.0.0.1:9090, falling back to 9091–9099. The toolbar shows the bound port, or why it couldn't bind.
- Synthetic data is opt-in: `PROXYMATE_SYNTH_FLOWS` seeds flows and `PROXYMATE_SYNTH_RATE` adds that many live ones per second. Both default to 0. Keep `synth` for tests and benches.
- Live segment files live under `~/Library/Application Support/Proxymate/live/<uuid>/`. Clear and quit delete them; the next launch sweeps any left by a crash.
- HTTPS: the CA is `<data dir>/ca/proxymate-ca.pem` (key `proxymate-ca-key.pem`, mode 0600) and the SSL lists are `<data dir>/ssl.json`. Upstreams are verified with the OS trust store plus `EngineConfig::extra_upstream_roots` (`PROXYMATE_EXTRA_UPSTREAM_CA` in the app and CLI). Give each test engine its own `data_dir` so tests don't share a CA.
- Integration tests are in `crates/pm-core/tests/`, using `pm-testkit` upstreams and clients. Tests that need their own process (RSS, environment variables) get their own file.

**Rules and breakpoints (M5)**
- One rule system: a `Matcher` plus ordered `Action`s (`pm-model/src/rules.rs`). Presets only pre-fill rules (`Engine::draft_rule`). The evaluation order is in `docs/PLAN.md` and `rules/eval.rs`; its unit matrix is `rules/tests.rs`, the end-to-end matrix `crates/pm-core/tests/rules.rs`.
- Every non-WebSocket request goes through `proxy/exchange.rs`. With no matching rule it streams exactly as before; keep that path allocation-free.
- Rules live in `<data dir>/app.sqlite` (in memory without a data dir, so tests never share rules). Mutations go through `Engine::change_rules`: validate, save, swap the `CompiledRuleSet`.
- Patterns compile with a size limit; never compile a user or agent regex without `rules::compile::compile_pattern`.
- A service error (`proxy::Reset`) is how a Block reset or a breakpoint Abort reaches the client: h1 closes, h2 resets the stream.

**MCP (M7)**
- The server is an adapter: tools call only `Engine`. MCP state the UI also sees (settings, token, audit log, connected clients) lives in `pm-core/src/mcp.rs`; the server reports serving/clients there.
- Add a tool in `tools.rs`: a parameter struct deriving `JsonSchema` (reuse `pm-model` types), a `def!` entry with its kind (`Read`, `Write`, `Destructive`: read-only mode hides every non-`Read`) and a `call` arm. Return JSON; errors the agent should read are `ToolError`.
- Filters are the DSL in `store/query.rs` (`FlowQuery::parse` strict for MCP, `parse_lenient` for the search box).
- `PROXYMATE_MCP_PORT` moves the app's endpoint off 9095 (parallel dev instances). Every dev instance shares the token and audit log in `app.sqlite`.

**macOS integration (M4)**
- Tests never change the machine's settings. System proxy logic is tested in `pm-platform` against a fake `ProxyBackend`; trust tests only read the keychain. Real keychain and proxy changes need the user's OK.
- System proxy: `networksetup` per active service, with a backup in `<data dir>/sysproxy-backup.json` written first. The app spawns `proxymate --sysproxy-guardian <pid> --data-dir <dir>`, which restores after a crash or SIGKILL (about 0.25 s). A stale backup is restored at launch. Only settings that still point at us are put back.
- The client app is resolved per connection in the background (`client.rs`); flows that start earlier get a `ClientResolved` event. The Client column and the sidebar Apps show its name; `pmicon://localhost/<name>` serves its icon.
- The certificate wizard's trust check CONNECTs to `verify.proxymate.test`, which the proxy answers itself without dialling or recording a flow.
- A `host` filter of `*.example.com` matches `example.com` and every host under it (sidebar domain rows).

**Device setup (M9)**
- `pm-core/src/devices.rs` owns LAN access, the allow-list (the `devices` table and `lan` settings row in `app.sqlite`, through M5's `rules::db::AppDb` on its own connection) and device activity; `proxy/onboarding.rs` serves `/`, `/cert.pem`, `/cert.crt` and `/proxymate.mobileconfig` to requests addressed to the proxy (origin form, our own address, or `proxy.mate`).
- Every accepted connection goes through `Engine::admit` first: loopback and our own addresses pass, LAN IPs wait up to 30 s for the "Allow?" prompt. Tests can't make a non-local peer, so the hold/allow/deny logic is unit-tested on `DeviceRegistry` directly.
- LAN access rebinds to `0.0.0.0` on the same port. Use `Engine::proxy_addr()` (loopback) for anything this machine connects to, and `display_addr` for what the user sees.
- Tests never change the machine: no simulator boot or keychain install, no emulator proxy. Real `simctl`/`adb` runs need the user's OK.

**Storage limits and deep search (M10)**
- `StorageSettings` (flows, disk MB, WebSocket messages per connection) and `ProxySettings` (port, plain-HTTP pool) are `app.sqlite` settings rows, owned by `pm-core/src/settings.rs`, which also runs eviction after each ingest batch (`Engine::evict_locked` → `Store::evict` → `DetailStore::evict` → `SegmentStore::drop_through`).
- Pinned (`FlowSummary.pinned`, `FlowRow`'s last field) and in-flight flows are never evicted. The arena may have gaps after an eviction: look flows up with `Store::get`, never by `id - first_id`.
- Every `SegmentStore::append` names the flow it writes for; that is how a segment knows when it can be deleted.
- Deep DSL terms (`header:`, `body:`, …) live in `store/deep.rs`. `FlowQuery::matches` passes them; anything evaluating a query over summaries must also run `FlowQuery::deep()` against the detail (views do it in the background job, MCP inline).

**Replay, cURL, HAR and sessions (M8)**
- `Engine::send(RequestSpec, SendOpts)` is the one way Proxymate sends a request itself (Replay, Compose, MCP). It goes through `proxy/exchange.rs` like proxied traffic; the flow's summary gets `sent_by`.
- Flow ids are per store. The live capture is `LIVE_STORE` (0); every opened session or HAR is a document store. Flow-level Engine methods have `*_in(store, …)` forms (the live-only ones stay for existing callers); view ids already encode their store. UI code reads the active store from `useUi().store` and puts it in query keys (`flowKey(store, id)`).
- Golden files are in `crates/pm-core/tests/golden/` (cURL and HAR); rewrite them with `UPDATE_GOLDEN=1 cargo test -p pm-core --lib io::`. The Chrome HAR fixture is `crates/pm-core/tests/fixtures/chrome-devtools.har`.
- A session's postcard layouts are part of its format: bump `store/sqlite.rs` `VERSION` when `FlowDetail` or the message types change shape.
