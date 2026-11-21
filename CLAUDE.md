# Proxymate

A fast desktop HTTP(S) debugging proxy (in the vein of Proxyman / Charles), built with Tauri 2, a Rust core and a React + TypeScript UI. macOS first, then Linux and Windows.

**`docs/PLAN.md` is the source of truth** for architecture, scope and milestones. Read it before starting work. If your work changes the plan, update `docs/PLAN.md` in the same PR.

## Delivery workflow (mandatory)

- Each milestone (M0–M21 in `docs/PLAN.md`) is built in **its own Claude session** and lands as **its own PR**. The review track (R0–R4, same file) follows the same rules on `rN-<slug>` branches.
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
| SOCKS5 through the proxy (M18) | `curl --socks5-hostname 127.0.0.1:9090 --cacert ~/Library/Application\ Support/Proxymate/ca/proxymate-ca.pem https://example.com/` (same port as HTTP) |
| Headless proxy | `cargo run -p proxymate-cli -- serve [PORT]`. Logs each finished flow. `--mcp[=PORT]` also serves MCP (prints the `claude mcp add` line) |
| MCP Inspector against the app | `npx @modelcontextprotocol/inspector --cli http://127.0.0.1:9095/mcp --transport http --header "Authorization: Bearer $(sqlite3 ~/Library/Application\ Support/Proxymate/app.sqlite "select json from settings where key='mcp_token'" \| tr -d '"')" --method tools/list` (add `--strict` for schema portability) |
| Stdio MCP bridge | `proxymate mcp-stdio` (app binary) or `cargo run -p proxymate-cli -- mcp-stdio [--port N] [--no-launch]` (token from `PROXYMATE_DATA_DIR`, else the app's data dir) |
| Test upstream by hand | `cargo run -p pm-testkit --bin upstream`, then `curl -x http://127.0.0.1:9090 http://127.0.0.1:<port>/json`. It also serves a WebSocket echo at `/ws` and a slow event stream at `/sse?count=N&interval_ms=M` |
| WebSocket client through the proxy | `cargo run -p pm-testkit --bin ws-client -- ws://127.0.0.1:<port>/ws` (CONNECT; `--absolute` for absolute form) or `wss://localhost:<port>/ws` against the TLS upstream. `--count`, `--interval-ms`, `--server-close` |
| TLS test upstream (h2 + h1) | `cargo run -p pm-testkit --bin upstream -- --tls` prints its port and CA file. Start the app with `PROXYMATE_EXTRA_UPSTREAM_CA=<that file>`, then `curl -x http://127.0.0.1:9090 --cacert ~/Library/Application\ Support/Proxymate/ca/proxymate-ca.pem https://localhost:<port>/json --http2 -v` |
| Rewrite dialog in the browser UI | `npm run dev`, then `http://localhost:1420/?zoo&select=0&rewrite=0`. In the app with `VITE_PM_REMOTE=1`: `{"rewrite":{"flow":3}}`, then `{"rewriteEdit":{"side":"response","json":[{"set":"/a","raw":"1"}],"save":true}}` (see `src/dev/remote.ts`) |
| Offline mocking in the browser UI (M20) | `npm run dev`, then `http://localhost:1420/?mocks&openapi&select=0` (a mock library entry, an imported spec, and a flow its `getItem` operation labels). Settings › Mocking; a tab's menu has "Mock from session…" |
| WebSocket rules in the browser UI (M19) | `npm run dev`, then `http://localhost:1420/?select=13` (an open mock WebSocket: composer, Resend, Edited/Injected/Dropped badges) or `?breakpoint=ws` (a held message in the Breakpoint Editor) |
| Capture targets in the browser UI (M21) | `npm run dev`, then `http://localhost:1420/?unrecorded&autosave` (a never-recorded host and footer count, auto-save on). The Intercept menu starts nothing in the mock. With `VITE_PM_REMOTE=1`: `{"noCache":true}`, `{"neverRecord":"example.com"}`, `{"recordingLists":{"only":[],"never":[]}}`, `{"autosave":{…}}`, `{"autosaveNow":true}`; `{"intercept":"chrome"}` really launches (user's OK only) |
| Open the app on a response tab | `VITE_PM_RESPONSE_TAB=TLS` (or `Timing`, …), for screenshots from a shell |
| Drive the app from a shell | Start with `VITE_PM_REMOTE=1`, then `curl -d '{"select":3,"requestTab":"Body"}' localhost:1420/__pm_remote` (also `responseTab`, `viewer: {kind, viewer}`, `theme`; M8: `replay`, `editReplay`, `compose`, `composeSend`, `store`, `openDocument`, `saveSession`, `exportHar`, and `curl`, whose answer is at `/__pm_remote/out`; M14: `scripts`, `scriptTemplate`, `scriptSource`, `saveScript`, `testScript`; M15: `{"contextMenu":{"testid":"request-headers","row":1}}` right-clicks any element; M17: `selectIds`, `selectAll`, `label`, `comment`, `headerColumns`, `compare`, `repeat`, `code`, `deleteSelection`; M20: `mockFrom`, `mockFromFile`, `importOpenApi`, `generateMocks`, `removeMockSource`, `removeOpenApi`, `deleteRule`, `rules`; M21: `intercept`, `neverRecord`, `recordingLists`, `autosave`, `autosaveNow`, `noCache`, `noCookies`, answered at `/__pm_remote/out`). See `src/dev/remote.ts` |
| gRPC upstream (M16) | `cargo run -p pm-testkit --bin upstream -- --grpc [PORT]` (tonic, TLS, `shop.v1.Orders`; prints its CA). Start the app with `PROXYMATE_EXTRA_UPSTREAM_CA=<that file>`, then `HTTPS_PROXY=http://127.0.0.1:9090 grpcurl -cacert ~/Library/Application\ Support/Proxymate/ca/proxymate-ca.pem -import-path crates/pm-testkit/proto -proto shop.proto -d '{"id":"o1"}' localhost.:<port> shop.v1.Orders/GetOrder` (`localhost.`: Go never proxies `localhost`) |
| Regenerate the zoo's binary fixtures and their decoded output | `UPDATE_ZOO=1 cargo test -p pm-testkit zoo` then `UPDATE_ZOO=1 cargo test -p pm-core --test zoo` |
| Body-viewer zoo through the proxy | `cargo run -p pm-testkit --bin upstream -- --zoo [127.0.0.1:9090]` sends one flow per fixture in `crates/pm-testkit/fixtures/zoo/` |
| Zoo in the browser UI | `npm run dev`, then open `http://localhost:1420/?zoo` (mock backend serving the zoo; `&select=<id>&theme=light&requestTab=Body`) |
| Viewer snapshots + preview safety (Playwright) | `npm run e2e` (first time: `npx -w @proxymate/ui playwright install webkit chromium`). `-- --update-snapshots` after an intended visual change; `PM_E2E_PORT` picks the dev server port (default 1425) |
| Open the app with the cert wizard / a search | `VITE_PM_OPEN=cert-wizard` (or `settings:mcp`), `VITE_PM_FILTER=<text>` (dev only, for screenshots) |
| Open the app on device setup | `VITE_PM_OPEN=device-setup:iphone` (or `simulator`, `android`, `emulator`). With `VITE_PM_REMOTE=1`, `{"deviceTab":"android"}` switches tabs and `{"click":"<data-testid>"}` clicks a control (e.g. `simulator-install`, `lan-access`) |
| Allow prompt in the browser UI | `npm run dev`, then open `http://localhost:1420/?devicePrompt=192.168.1.42` (mock LAN client waiting to be allowed) |
| Run the app as the system proxy | `PROXYMATE_SYSTEM_PROXY=1 npm run tauri dev`. Changes the real macOS proxy settings: restored on quit, on signals, and by the guardian after `kill -9` |
| UI only, in a browser | `npm run dev` (http://localhost:1420, in-memory mock backend) |
| Rust format / lint / test | `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (CI runs `cargo nextest run --workspace --profile ci`: per-test timeouts in `.config/nextest.toml`) |
| Dependency audit (CI's `audit` job) | `cargo deny --locked check` (`deny.toml`; `cargo install --locked cargo-deny`) and `npm audit --omit=dev` |
| UI typecheck / lint / test / build | `npm run typecheck`, `npm run lint`, `npm run test`, `npm run build` |
| Regenerate TS bindings | `cargo test -p proxymate-desktop export_bindings`. Debug app launches also regenerate them |
| Engine micro-bench | `cargo run --release -p proxymate-cli -- bench [FLOWS]` |
| Soak (M10) | `cargo run --release -p proxymate-cli -- bench --rps 500 --duration 3600 [--max-flows N]` (own engine), or `--proxy 127.0.0.1:9090 --pid <app pid>` against the app. CSV every `--interval` seconds: rate, p50/p99, failures, RSS, flows, disk, evicted |
| Hot path bench (criterion: ingest < 2 s per 100k flows, sorted views, eviction, refilter/sort < 10 ms, deep search) | `cargo bench -p pm-core --bench ingest` |
| Bench budget (CI's `bench` job) | `PM_BENCH_QUICK=1 cargo bench -p pm-core --bench ingest && scripts/bench-budget.py` (ceilings in `crates/pm-core/benches/budget.json`) |
| Release bundle | `npm run tauri build` (host arch, ad-hoc signed) |
| macOS release build (universal DMG, checks, staging) | `scripts/release/macos.sh` (needs `rustup target add x86_64-apple-darwin`; ad-hoc unless the signing variables in `docs/RELEASING.md` are set; output in `target/release/macos`) |
| Native app (gpui) | `cargo run -p proxymate-native`. Same data dir, env vars (`PROXYMATE_SYNTH_*`, `PROXYMATE_EXTRA_UPSTREAM_CA`, `PROXYMATE_MCP_PORT`) and binary modes as the Tauri app; `PROXYMATE_DATA_DIR=<dir>` runs a dev copy beside the installed app (one copy per data dir) |
| Drive the native app from a shell | `PROXYMATE_DEV_REMOTE=1 cargo run -p proxymate-native --features dev-harness`, then `curl -d '{"theme":"light"}' localhost:1430/__pm_remote` (also `{"screenshot":"/abs/x.png"}`, `{"click":"<element id>"}`; G1: `select` (row index), `selectIds`, `selectAll`, `scrollTo`, `headerColumns`, `autoSelect`, `table`; G2: `filter`, `search`, `protocol`, `content`, `scope`, `keys` (`"cmd-k"`), `palette`, `command`, `store`, `openDocument`, `closeDocument`, `saveSession`, `exportHar`, `recording`, `clear`, `contextMenu` (`"listening"`, `"tab:1"`, `"sidebar:app:Safari"`, … with `"run": "<label>"`), `sidebarFilter`, `workspace`; G3: `requestTab`, `responseTab`, `focus`, `curl`, `inspector`, and `contextMenu` names `url`, `kv:request-headers:2`, `rule:0`, `cert:0`, `timing`, `failure`, `raw:response`, `row:3:status`; G6: `streamSelect`, `streamDirection`, `streamSearch`, `streamViewer`, `wsOriginal`, `wsCompose`, `wsComposeSend`, `stream`, `wsSend`, `wsClose`, `tableShare`, and `contextMenu` names `stream:4`, `payload`; answered inline). `PROXYMATE_DEV_RESPONSE_TAB=TLS` opens the response pane on a tab. `PROXYMATE_DEV_FILTER=<text>` starts with search text. See `apps/native/src/dev/mod.rs` |
| Native perf probe (G1) | `PROXYMATE_SYNTH_FLOWS=100000 PROXYMATE_DEV_PERF_PROBE=1 cargo run --release -p proxymate-native --features dev-harness` (window on screen; `[perf-probe]` lines on stderr). `PROXYMATE_DEV_AUTO_SELECT=1` starts with Auto Select on. `PROXYMATE_DEV_*` are read only in debug builds or with `dev-harness` |
| Native shell rendered offscreen | `cargo test -p proxymate-native --test render` (both themes, pixels checked, PNGs in `target/native-snapshots/`) |
| Native spikes (G0) | `cargo run --release -p proxymate-native --features spikes --example spike_bench [a\|e]`; `--example spike_native_view -- <capture.png>` (macOS) |
| Updater end to end | Debug bundle (`npm run tauri -- build --debug`) launched with `PROXYMATE_UPDATE_TEST=1`: checks at launch and installs without asking. Recipe in `docs/RELEASING.md` |
| Native release bundle (G15) | `cargo xtask bundle-macos` (universal; `--arch host --debug --version V --no-dmg` for a quick local one; the updater key and signing variables as for `macos.sh`; output in `target/release/native-macos`, the app in `target/native-bundle/<profile>/`). `cargo xtask updater-key <path>` makes a throwaway key pair |
| Native updater end to end (G15) | A Tauri 0.6.x debug app and a native debug bundle (`PROXYMATE_UPDATER_ENDPOINT`/`PROXYMATE_UPDATER_PUBKEY` at build time) against a local feed, launched with `PROXYMATE_UPDATE_TEST=1`. Recipe in `docs/RELEASING.md` |

CI (`.github/workflows/ci.yml`):
- Rust: `fmt --check` and `clippy -D warnings` on Linux (`fmt + clippy`); `cargo nextest` and the bindings-drift check on macOS and Linux (`test (…)`); the R2 `bench budget` on Linux. These exclude the gpui crates (`proxymate-native`, `pm-widgets`).
- Native (G0): `native (…)` on macOS and Linux: clippy with all features and nextest for `proxymate-native`, `pm-widgets`, `pm-theme`, `pm-view`, `pm-app`, then `tests/render.rs` (Metal; WGPU on Mesa's lavapipe on Linux), its PNGs uploaded as `native-snapshots-<os>`. Linux needs gpui's libraries (`setup-rust`'s `native-deps`).
- Audit: `cargo deny check` (advisories, yanked crates, licences, duplicates as warnings, sources) and `npm audit --omit=dev`. A finding is fixed (`cargo update -p <crate>`) or ignored in `deny.toml` with its reason.
- UI: `typecheck`, `lint`, `vitest`, `build` and Playwright (`e2e`), in the Playwright Docker image with pixel diffs on. Linux baselines (`*-linux.png`) are made there: a missing one fails the run and comes back in the `playwright-report` artifact to commit. The image tag and `@playwright/test` move together, by hand (Dependabot ignores Playwright).
- `ci` is the one required check: it passes when every job passed or was skipped. A pull request that only touches `docs/`, root `*.md` or `LICENSE*` skips the build jobs.
- (R0) The toolchain is pinned in `rust-toolchain.toml` (CI and releases install it with `rustup toolchain install`); bump it in its own PR, since a new stable brings new clippy lints. Actions are pinned by commit SHA with the version in a comment; Dependabot (`.github/dependabot.yml`) updates them, cargo and npm weekly. Workflows get `contents: read` and every job a `timeout-minutes`; a new push cancels a pull request's older run, never main's.

Run the same checks locally before pushing.

## Layout and crate boundaries

```
crates/pm-model      pure types (serde + specta): FlowSummary, FlowRow, FilterSpec, SortSpec, ViewDelta, ...
crates/pm-core       headless Engine: proxy/ (h1 forward proxy, mitm.rs: CONNECT, MITM, h2, passthrough, ws.rs: WebSocket
                     relay with message rules, breakpoints and ws_send (M19), send.rs: Engine::send, dial.rs: the one dialer, socks.rs: SOCKS5 listener, reverse.rs: reverse
                     proxies), network.rs (M18: Settings › Network compiled, secrets, reverse listeners), tls/ (CA, leaf cache, SSL policy, upstream verifier), recorder
                     (per-flow capture), ingest (event thread), store/ (arena + views, sidebar counts, backend.rs:
                     BlobBackend, segments, sqlite.rs: session tables + SqliteBackend, bodies, details, messages: WS/SSE
                     logs), io/ (content kind, decoding, sse parser, curl, har, session), documents (opened sessions and
                     HARs), replay, client (loopback peer → app), trust (handshake check), integration (the OS-facing
                     Engine methods), rules/, scripts/ (M14: QuickJS workers, prelude.js, native.rs, proxymate.d.ts), synth,
                     protos.rs (M16: schemas, mappings, Engine::decode_body_in), io/grpc.rs (framing, status), io/wire.rs
                     (schema-less protobuf), io/objects.rs (MessagePack, CBOR), io/tree.rs (ordered JSON + badges),
                     workflow.rs (M17: delete, annotations, header columns, Repeat), io/snippets.rs (Copy as code),
                     mocks.rs (M20: the mock library and ServeFrom's index), openapi.rs (specs, operations, mocks),
                     io/charles.rs (Charles .chlsj/.chlsx import), capture.rs (M21: recording lists,
                     intercepted launches, auto-save)
crates/pm-platform   OS integration traits; macos/ impls, stub.rs elsewhere: fs (private files), sysproxy (backup +
                     restore over a ProxyBackend; networksetup; the previous proxy), trust (keychain), secrets (M18:
                     SecretStore: keychain, 0600 file, memory), procinfo (libproc port scan, app
                     bundles, icons, parent pids), launcher (M21: Terminal/iTerm, Chromium, Firefox), process (kqueue exit wait), netif (if-addrs), simctl and adb (over a
                     command::Runner, faked in tests)
crates/pm-mcp        MCP server over Arc<Engine> (rmcp, pinned): http.rs (axum endpoint, bearer token, managed start/stop),
                     handler.rs (sessions, read-only, audit, notifications), tools/ (by domain), resources.rs, prompts.rs,
                     bridge.rs (mcp-stdio); tests/mcp.rs drives an rmcp client against a headless engine
crates/pm-testkit    test upstreams (h1, TLS h1/h2 with a test CA, mTLS, WS echo, slow SSE, grpc.rs: tonic gRPC + gRPC-web
                     from proto/shop.proto via build.rs), proxies.rs (M18: HTTP proxy with Basic auth, SOCKS5 server and
                     client), the body-viewer zoo (fixtures/zoo), proxy, HTTPS, gRPC and WebSocket
                     clients; re-export of pm_core::synth
apps/desktop/src-tauri   thin Tauri adapters (commands/, one module per domain), the pmbody:// scheme (body_scheme.rs), window chrome (window.rs),
                         the menu bar (menu.rs: File items reach the UI as `pm-menu` events); updater.rs (tauri-plugin-updater,
                         Check for Updates…, the launch check)
apps/desktop/ui          Vite + React 19 + TS strict + Tailwind v4 + shadcn/ui
apps/cli                 proxymate-cli (headless; `bench` today)
crates/pm-app        host glue both apps share: binary modes (guardian, mcp-stdio), start_engine, exit guards, MCP spawn,
                     data dir, single instance (native)
crates/pm-theme      colour tokens read from ui/src/styles/tokens.css at build time (build.rs + css.rs), the contrast test
                     (contrast.rs), the gpui-kit mapping and ActiveTokens global (feature `kit`)
crates/pm-view       native UI logic with no gpui (ports of lib/*.ts with their tests): selection, keys, columns +
                     TablePrefs, flow formatting, RowCache (G1); DSL autocomplete, copy formats, the command list (G2);
                     errors (explain_failure), http, tls, inspector menus and Raw rules, sample fixtures (G3); stream (the
                     WebSocket and SSE panels' logic, G6)
crates/pm-widgets    reusable gpui widgets: Pill, flow_grid/ (FlowGrid: rows, header, icons; gpui tests), Assets
                     (gpui-kit's plus the Lucide icons the widgets draw), KvTable (G3); HexView, … as phases land
apps/native          the native app (gpui + gpui-kit, binary `proxymate-native`): bridge/ (runtime, engine, streams; G3:
                     resource caches, keys, mutations), shell/
                     (main window, chrome), appearance, prefs (ui-prefs.json), menus, features/ (one module per area),
                     dev/ (remote, feature `dev-harness`), updater/ (G15: feed and signatures, install); resources/macos
                     (entitlements, icon); tests/ (render.rs offscreen, shell.rs, inspector.rs); examples/ (G0 spikes)
xtask                `cargo xtask` (G15): bundle-macos (the native Proxymate.app, DMG, signed updater archive,
                     updater-darwin.json, macos.sh's checks), updater-key
brand                    the Detour logo: masters generated by src/build.py, PNGs, web icons, guidelines (README.md)
```

Rules:
- **Adapters call only the `Engine` facade** (`pm_core::Engine`). Tauri commands, MCP tools and the CLI never reach into `pm_core::store` or other internals. This keeps the UI and agents behaving the same. If an adapter needs something new, add it to `Engine`.
  - (R3) `pm-core`'s modules are `pub(crate)`, so the compiler enforces this. Its integration tests, benches and `pm-testkit` use the hidden `pm_core::test_support` door; adapters never do.
- `pm-core` has **no Tauri dependency** and stays headless and testable.
- `pm-model` holds data types only: no I/O and no engine logic. Every type that crosses IPC lives here and derives `Serialize`, `Deserialize` and `specta::Type`.
- OS-specific code goes in `pm-platform` behind traits, never `cfg`-sprinkled through `pm-core`.

## Data path (performance core)

1. Traffic handlers record through a `FlowRecorder` (`Engine::begin_flow`). Summary changes go as small `FlowEvent`s over one channel to the `pm-ingest` thread, which applies each batch (up to 5 ms) under one write lock. Size updates (Progress, WebSocket message sizes) don't queue: `IngestTx::send` folds them into one pending update per flow behind a single `FlowEvent::Sizes` marker. Views update incrementally: a sorted view settles the rows a batch touched with binary insertion (`View::settle`), and its delta is structural only if a row the UI has moved. Bodies never travel in events: they are tee'd into `store/blob.rs` (inline up to 8 KB, else segment files, capped at 64 MB, still content-encoded). Segment I/O runs on the `pm-segments` thread: `SegmentStore::append` only reserves the extent and queues the bytes (bounded, about 16 MB), and reads wait for the writes before them. `Engine::flush` waits for both.
2. The publisher thread ticks at 20 Hz and sends one `ViewDelta` per subscribed view over a `tauri::ipc::Channel`. A delta carries `{total, len, appended, changed ids, structural}`. Stores are `SharedStore`s: every write guard bumps a generation, and a tick skips a store nothing wrote to (`write_quiet` is for bookkeeping the publisher needn't see). The sidebar goes out at most every 250 ms.
3. The UI (`hooks/useFlowView.ts` and `lib/rowCache.ts`) fetches only the visible window plus one page of read-ahead with `get_rows(view, start, count)`, coalesced to one call per run of adjacent pages per animation frame. Rows arrive as positional `FlowRow` tuples, decoded by `lib/flow.ts`. `get_rows` returns at most 512 rows (`MAX_ROWS_PER_CALL`): ask in steps for more.
4. `set_filter` and `set_sort` refilter synchronously in Rust (rayon above 50k flows) and return the new length right away. Search-box typing is debounced (`FILTER_DEBOUNCE_MS`); chips and scopes apply at once.
5. Tauri commands that touch the store, bodies or disk are `async` and run on the blocking pool (`blocking` in `commands/mod.rs`): a sync command runs on the main thread. MCP runs every sync tool on the blocking pool (`call_sync` in `tools/mod.rs`). Both go through `Engine::blocking`.

Targets:
- 100k rows scroll at 60 fps.
- A filter change repaints in under 50 ms end to end.
- An engine refilter takes under 10 ms.
- `FlowRow` field order is part of the IPC contract. If you change it, update `decodeRow` and its test.

## Conventions

**Rust**
- Edition 2024, with `rustfmt.toml` at `max_width = 120`.
- Clippy must pass with `-D warnings`. `unsafe_code` is denied workspace-wide; scope any exception with `#[allow]` and a justification.
- Errors use `thiserror` in library crates. Tauri commands fail with `pm_model::IpcError` (`{kind, message}`): `pm_core::Error::kind()` gives an engine error's `IpcErrorKind`, anything else is `internal` (`ipc` in `commands/mod.rs`). The UI's `unwrap` throws a `BackendError` with that `kind`; switch on it, never on the message.
- Each module has a short `//!` doc. Comments explain *why*, not *what*.

**TypeScript**
- `strict` and `noUncheckedIndexedAccess` are on. Import with the `@/` alias.
- Use the generated `src/bindings.ts` types. Never edit that file by hand, never hand-write IPC types, and call commands through `lib/backend.ts`.

**State**
- Zustand (`store/ui.ts`) holds UI state; `store/prefs.ts` holds what persists across launches (appearance, columns, sort).
- Row data lives in the non-reactive `RowCache`, never in React state.
- TanStack Query holds server state (flow detail, bodies, rules, settings): one `QueryClient` and the key factories in `lib/query.ts`. Keep one shape per key.
- (R4) Every query key lives in `lib/query.ts`, one root per shape (`query.test.ts` checks it). Keys of engine data are typed with `DataTag`, so `getQueryData`/`setQueryData` need no type arguments. Fetch through `lib/queries.ts` (`useQuery(queries.proxyStatus())`, or spread it and add `enabled`): one fetcher per key, passing TanStack's `signal`.
- (R4) Writes that change cached state are `lib/mutations.ts` entries with their cache effect (a setter stores its answer, the rest invalidate what they made stale). Components run them with `useMutation`, non-React code with `runMutation`. Don't call `setQueryData`/`invalidateQueries` after a backend write by hand.
- (R4) `Backend` reads that queries make take an optional trailing `AbortSignal`; `lib/abort.ts` (`withSignals`, `SIGNAL_ARG`) applies it to both backends. A new read that a query makes gets a `signal?` parameter and a `SIGNAL_ARG` entry.
- (R4) Dev-only setup lives in `src/dev/`: `harness.ts` (env vars, URL parameters, the remote, the perf probe; `main.tsx` calls it only in dev) and `bridge.ts`, where editors register buttons for `remote.ts` with `useBridge({ … })`.

**Design system: shadcn/ui** (Radix + Tailwind v4)
- The components are copied into `src/components/ui/` and restyled to a compact density: 12–13 px text and 24–28 px controls. Add more with `npx shadcn@latest add <name>` from `apps/desktop/ui`.
- Check the result: once the CLI rewrote the `cn` import to a stray npm package.

**Colours and themes**
- Every colour is a token in `src/styles/tokens.css`: shadcn variables plus `--m-*` (methods), `--s-*` (status and state), `--url-*`, `--selection-*` and the rest. Don't hard-code colours in components (not even `bg-white` or `bg-black/50`: there are `--paper` and `--overlay`).
- Both themes are first-class. `src/styles/contrast.test.ts` checks 4.5:1 for every text token on its surface (pills composited over the table) in dark and light, and 3:1 for icon-only colours (`GRAPHICS`: the record button's `--record-start`/`--record-stop`). It scans `tokens.css` (R4): a new token fails it until it has a pair, or is a surface or line listed in `NOT_TEXT`.
- Settings › Appearance (System/Light/Dark) lives in `store/prefs.ts` (local storage) with the column layout and sort; `lib/theme.ts` applies it and syncs the native window theme (`setTheme`, `null` for System), which the macOS sidebar vibrancy and traffic lights follow. `?theme=light` (and the remote's `theme`) pins a theme in dev (Playwright) and sets the window theme too.

**Body viewers** (`src/components/BodyViewer/`)
- `registry.ts` maps each `ContentKind` to its viewers, best first. Add a viewer there and in `Viewer.tsx`.
- Media, images, PDFs and fonts load from `backend.bodyUrl` (`pmbody://` in the app, blob URLs in the mock); text viewers get decoded bytes over IPC.
- Big work runs in workers: JSON parsing/formatting above 256 KB, and prettier (lazy-loaded) for Beautify.
- The HTML preview must stay sandboxed (no `allow-scripts`), URL-stripped and under `PREVIEW_CSP` (`lib/body/htmlPreview.ts`); `e2e/security.spec.ts` checks it.

**The flow table**
- It is a custom div grid virtualised with TanStack Virtual, at a fixed 22 px row height. Do not swap in shadcn's DataTable.
- Keep row components memoised, with stable callbacks.
- Columns are defined in `FlowTable/columns.ts`; a new one also needs a `RowCell` case (`FlowRow.tsx`) and a place in `DEFAULT_LAYOUT` (saved layouts pick it up at its default position).
- (R4) The table is an ARIA grid: rows of `gridcell`s, the header's row is `aria-rowindex` 1, the grid keeps the focus and names the focused row with `aria-activedescendant`. A sortable header's title is a button; resize handles are focusable separators (←/→, Home/End, ↩). Its keys are the pure `FlowTable/keys.ts` (tested in `keys.test.ts`); the grid ignores keys aimed at the header's controls.

**Accessibility (R4)**
- `eslint-plugin-jsx-a11y-x` lints (`recommended`; the ESLint 10 fork of jsx-a11y). A disable says why on the same line.
- `DialogContent` and `SheetContent` require `initialFocus`: `"content"` (the dialog itself) or a selector inside it (`[data-autofocus]` on the field). Never `autoFocus`, and never leave focus on the window.
- `e2e/a11y.spec.ts` runs axe (WCAG 2.1 A/AA) on the main window, the zoo and every dialog in both themes. A new dialog gets a case there. `PM_A11Y_DUMP=<file>` appends full reports while fixing.

**Context menus and copying (M15)**
- Copy through `lib/clipboard.ts` (`copy`, `copyLater` for text that is fetched first, `copyImage`); never call `navigator.clipboard` directly. Text formats (header lines, cookies, hex, Base64, filter terms) live in `lib/copyFormats.ts` with their tests.
- Menus are built from `components/menus/`: `CopyMenu` for one element, `useMenuTarget` plus `data-menu-key` for a list (one menu per list, not per row), `SelectionItem` first wherever text is selectable, `BodyItems` at the end of a body viewer's menu (`BodyCopyContext` says which body).
- Copy the whole value, not what's displayed when it's truncated. The native menu policy (`hooks/useNativeMenuPolicy.ts`) only acts in release builds.

**Keyboard and commands (M10)**
- `lib/commands.ts` is the single list: the ⌘K palette shows every command, and a command's `key` binds ⌘/Ctrl+key in `hooks/useShortcuts.ts`. Add commands there, not ad-hoc `keydown` listeners. ⌘⌫ never fires in text fields; ⌘F inside CodeMirror is left to its search.
- Errors from actions go to `reportError` (a sonner toast); a failed flow's explanation and fixes come from `lib/errors.ts`, which switches on `FlowDetail.error_kind` (R3), never on the message text.

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
- Rules and rule groups live in `<data dir>/app.sqlite` (`rules`, `rule_groups`; in memory without a data dir, so tests never share rules). Mutations go through `Engine::change` (`change_rules` for the list alone): validate, `arrange`, save both in one transaction, swap the `CompiledRuleSet`, push a `RuleBook`.
- A rule runs while it and its group are enabled (`Rule::is_active`, never `rule.enabled` alone when compiling). `arrange` keeps the list in sections (built-ins, ungrouped, then group by group) and renumbers `order` after every change, so a saved rule's `order` can move: read it back from the change, not from what was passed in.
- Patterns compile with a size limit; never compile a user or agent regex without `rules::compile::compile_pattern`.
- A service error (`proxy::Reset`) is how a Block reset or a breakpoint Abort reaches the client: h1 closes, h2 resets the stream.

**MCP (M7)**
- The server is an adapter: tools call only `Engine`. MCP state the UI also sees (settings, token, audit log, connected clients) lives in `pm-core/src/mcp.rs`; the server reports serving/clients there.
- Add a tool in its domain module under `tools/` (flows, websocket, rules, scripts, send): a parameter struct deriving `JsonSchema` (reuse `pm-model` types) and a handler there; a `def!` entry with its kind (`Read`, `Write`, `Destructive`: read-only mode hides every non-`Read`) and a `call` (async) or `call_sync` arm in `tools/mod.rs`. Return JSON; errors the agent should read are `ToolError`. Rules change through `Engine` in one step (`patch_rule`, not read-modify-write).
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

**Native app (G0–G16)**
- `docs/PLAN.md` "Native (gpui) track" is the plan, with the G0 spike results; `docs/native-parity.md` is the ledger each phase ticks with evidence (a test, or a dev-remote screenshot in its PR).
- gpui-kit is pinned `=0.10.0`, the one source of gpui (never add a git `gpui`). Bumps are their own "Bump gpui-kit" PRs. Its `test-support` makes the lock include `gpui-fast`, which pins `wgpu =30.0.1`.
- Adapters call only the `Engine` facade, as before. File dialogs run in the app (`cx.prompt_for_paths`), and the path goes to the Engine.
- The bridge (`apps/native/src/bridge/`) owns the tokio runtime and `Arc<Engine>`. Engine sinks only enqueue (`bridge::stream`); one foreground task per stream drains into an entity and notifies. Single-slot streams are subscribed once (`bridge::Streams`) and shared by every window. Blocking engine calls go through `Engine::blocking` or `cx.background_spawn`, never on the foreground thread (O(1) getters excepted).
- Views take `pm-model` snapshots (entities) in their constructors, so `tests/` render them with no engine (see `tests/render.rs`). A new viewer or dialog gets an offscreen render case.
- Every colour comes from `pm-theme` (`cx.tokens()`, `Rgba::hsla()`), never a literal. `pm_theme::kit::apply` sets gpui-kit's theme from the tokens. Change colours in `tokens.css` (both apps read it until G16); `pm-theme`'s contrast test runs on it.
- Interactive elements are gpui-kit controls, or set an accessibility role and label (G0 spike d: plain divs are invisible to VoiceOver). Give anything the dev remote clicks an `.id(…)` (and `.test_support()` on plain elements).
- Each feature module has `register(cx)` for its actions, keys, palette entries and menu items; `features/mod.rs` only lists modules.
- AppKit code (vibrancy, captures, later PDFKit/AVKit) lives in `pm-platform` (`window.rs`, `macos/window.rs`, one scoped `#[allow(unsafe_code)]` for the raw NSView). gpui draws under native child views: G5's host hides them while a gpui overlay is open.
- The main window's gpui-kit `Root` is transparent on macOS (vibrancy); every pane paints its token. Traffic lights are at gpui (16, 17), the Tauri app's (16, 22).
- Prefs (appearance; the table's `columns`, `sort` and `headerColumns`, spelt as `store/prefs.ts` spells them) are `<data dir>/ui-prefs.json`, never the WebView's storage. `prefs::update` writes 300 ms later (a burst writes once) and `prefs::flush` on quit.
- Shell chrome (G2): the shared state is `workspace::Workspace` (active tab, documents, filter, save job, proxy status, SSL lists), one entity per app (`Workspace::global`); the table follows its events (text changes debounced 120 ms, chips and scopes at once, tab switches make a new view). File dialogs live in `features::documents`.
- Commands (G2): `pm_view::commands` is the list (ids, titles, groups, keys, from `lib/commands.ts`); a feature registers a handler per id with `features::commands::add(id, enabled, run)` in its `register`, and `bind_keys` (last in `features::register`) binds the keys of registered commands. The ⌘K palette lists what has a handler and applies now. App-level action handlers that need the window defer to after the dispatch (`cx.defer`).
- Menus and copying (G2): context menus are `features::context_menu::Menu` values built by plain functions from state (one builder per list, keyed by row, e.g. `shell::sidebar::menu_for("app:Safari")`) and shown as a `NativeMenu`; copy entries dispatch `clipboard::CopyText`. Copy through `features::clipboard` (`copy`, `copy_later`, `copy_image`), never `cx.write_to_clipboard` directly; errors from actions go to `clipboard::report_error` (a toast). Tests run a menu's entries with `Menu::run` and read the test clipboard.
- Server state (G3): read engine state on demand through a `bridge::resource::Resource` cache whose key lives in `bridge/keys.rs` (one value shape per key; `keys::caches`), and write through `bridge/mutations.rs`, which invalidates what each write makes stale. Push streams invalidate too (`keys::flows_changed` on the table's deltas). Never call an engine write from a view.
- Stream panels (G6, `features/inspector/streams/`): one `StreamPanel` entity per inspected WebSocket or SSE flow, reading list heads and 128-row pages through the `ws_*`/`sse_*` caches (`bridge/keys.rs`); the table's deltas make a flow's heads and partial pages stale (`keys::streams_changed`), so nothing polls. Sends, Resend and Close are `bridge/mutations.rs` writes. Payloads show as Text or Hex until G4's viewers are wired in.
- The inspector (G3, `features/inspector/`): G4 fills the Body tabs (`panes.rs` `body_tab`) and moves Raw onto CodeView; buttons for later dialogs dispatch the stubs in `features::chrome`, which those phases replace. The flow row menu (`features/flow_menu.rs`) takes later items through `flow_menu::extend(Slot, …)` and the bulk menu through `set_bulk`, never by editing the menu. Selectable text is gpui-kit's `SelectableText`; menus over it start with `Menu::selection(context_menu::selected_text(…))`.
- The flow table (G1): `pm_widgets::flow_grid::FlowGrid` knows no engine. Its owner (`features/flow_table.rs`) gives it a `RowSource` (a blocking `get_rows`, run on the background executor, at most 512 rows a call) and the view's deltas; rows are formatted once when their page lands (`GridRow`). `uniform_list` also calls the row callback for row 0 alone to measure it: that call is not the visible window. Selection is ids (`pm_view::selection`), keys are `pm_view::keys`; ⌘A and ⌫ are `flow_grid::SelectAll`/`DeleteSelection` in the `FlowGrid` key context.

**Release and updates (M11)**
- `docs/RELEASING.md` covers secrets, cutting a release (tag `v*` → draft GitHub Release) and key rotation. `.github/workflows/release.yml` runs `scripts/release/macos.sh`.
- Releases are **ad-hoc signed and not notarised** (no paid Apple Developer account); users allow the app once past Gatekeeper (`scripts/release/install-notes-macos.md`, added to each draft). Developer ID signing and notarisation stay dormant until the `APPLE_*` secrets exist.
- Hardened runtime with no App Sandbox and an empty `Entitlements.plist`. Don't add an entitlement without its reason in RELEASING.md.
- The guardian and the MCP bridge are modes of the one signed binary. A sidecar would need signing too.
- Bump the version in `tauri.conf.json`, the workspace `Cargo.toml` and `apps/desktop/ui/package.json` together. The release tag must match `tauri.conf.json`.
- The updater lives in `src-tauri/src/updater.rs` (Rust-side; no JS plugin permissions). Its public key is in `tauri.conf.json`; the private key never enters the repo.
- The updater turns on reqwest's `rustls-no-provider` for the whole workspace. Any reqwest/rmcp client (the stdio bridge, MCP tests, the native updater) must call `pm_mcp::install_crypto_provider()` before it's built, or it panics.
- (G15) The native app is bundled by `cargo xtask bundle-macos` in the Tauri release's formats: the same bundle id, executable name (`Contents/MacOS/proxymate`), `Info.plist` keys, entitlements (`apps/native/resources/macos/Entitlements.plist`), minisign key and `.sig` format (the trusted comment adds `version:`), file names and `updater-darwin.json`. `.github/workflows/native-bundle.yml` builds it universal with a throwaway key; `release.yml` keeps running `macos.sh` until G16.
- (G15) The native updater (`apps/native/src/updater/`) reads the same feed and trusts the same public key (`feed::PUBLIC_KEY`, tested equal to `tauri.conf.json`'s). It refuses a signature whose trusted comment names no version or another one. Install swaps bundles with `pm_platform::fs::swap` (`renamex_np`), and the restart is gpui's `restart`, after the quit callbacks.

**Scripts (M14)**
- `crates/pm-core/src/scripts/proxymate.d.ts` is the script API's source of truth. A new global or member goes in `prelude.js` (or `native.rs`), the `.d.ts`, `scripts/api_surface.txt` and the editor's `GLOBAL_COMPLETIONS` (`ui/src/lib/scripts.ts`); a Rust test and a vitest check they agree.
- Scripts get no I/O: no network, files, module loader or timers. Never enable rquickjs's `loader`, `futures` or `parallel`, and never load bytecode.
- `unsafe_code` has one scoped exception, `scripts/native.rs::bytes_of`: rquickjs 0.14 only exposes a typed array's bytes through `unsafe`; they are copied out before any JavaScript runs.
- `FlowRules.scripts` is the last field of `FlowRules` (M20 moved the last `Action` variant to `ServeFrom`, and `ScriptPhase::WsMessage` is that enum's last). Add fields or variants after them only with a session `VERSION` bump.
- Rule evaluation is async only for scripts: a rule set without scripts never awaits (`rules/tests.rs` drives it with `now_or_never`).
- Templates are `.js` files in `ui/src/lib/script-templates/`, imported raw by the UI and run by `scripts::tests::templates_run`.
- Saving a script evaluates it (top-level code runs, with the traffic timeout), so Tauri commands and MCP tools that save scripts run off the async threads.

**Replay, cURL, HAR and sessions (M8)**
- `Engine::send(RequestSpec, SendOpts)` is the one way Proxymate sends a request itself (Replay, Compose, MCP). It goes through `proxy/exchange.rs` like proxied traffic; the flow's summary gets `sent_by`.
- Flow ids are per store. The live capture is `LIVE_STORE` (0); every opened session or HAR is a document store. Flow-level Engine methods have `*_in(store, …)` forms (the live-only ones stay for existing callers); view ids already encode their store. UI code reads the active store from `useUi().store` and puts it in query keys (`flowKey(store, id)`).
- Golden files are in `crates/pm-core/tests/golden/` (cURL, HAR and M17's `snippets/`); rewrite them with `UPDATE_GOLDEN=1 cargo test -p pm-core --lib io::`. The Chrome HAR fixture is `crates/pm-core/tests/fixtures/chrome-devtools.har`.
- A session's postcard layouts are part of its format: bump `store/sqlite.rs` `VERSION` when `FlowDetail` or the message types change shape. Version 8 (R3: `FlowDetail.error_kind`; reading 7 and older infers the kind from the message with `failure::legacy_kind`) still reads 7 (M20: `PresetKind::ServeFrom` and `OpenApi`, which an applied rule's origin can name; no layout change), 6 (M19: WebSocket message flags and the `originals` table), 5 (M18: `FlowDetail.via`, the old message layout) down to 1 (`tests/fixtures/session-v1.proxymate`; `tests/fixtures/session-v7.proxymate` is a committed version 7 file that `store/sqlite/v7_fixture.rs` decodes field by field, and never regenerate it: it is the version 7 case, and a later layout change bumps `VERSION` again; the same file pins the postcard index of each enum a session stores by position (`ErrorKind` too), so a new variant must be appended there; `a_version_2_session_still_opens`, `a_version_4_session_still_opens`, `a_version_6_session_still_opens`, `a_version_7_session_still_opens` and `store::sqlite::tests::a_version_5_websocket_log_still_opens` make older files from a current one). `Action` itself is never in a session (only `AppliedRule` is), but `PresetKind` is: a new preset bumps `VERSION`.
- postcard can't deserialize internally tagged enums (`#[serde(tag = …)]`). Anything inside `FlowDetail` must not be one, or must serialize by hand like `RuleOrigin`.

**Visual rewrite (M13)**
- `JsonSet`/`JsonRemove` edit bodies in place through `rules/json_edit.rs` (the byte splicer): never parse and re-serialise a body to change it. `JsonSet.raw` (exact JSON text) wins over `value`. Don't turn on serde_json's `arbitrary_precision` or `preserve_order` (`docs/PLAN.md`, "M13").
- In the UI, JSON that will be edited goes through `lib/json/doc.ts` (ordered nodes, exact numbers, stable ids), not `JSON.parse`; edits become actions in `lib/rewrite/diff.ts` and `session.ts`, whose tests are the table to extend.
- Dry runs and presets work on any store: `draft_rule_in`, `test_rule_in`, `flows_matching_rule`.
- The JSON editor handles its own keys (`[data-json-editor]` counts as editable for global shortcuts); a dialog holding it passes `keepEscapeInEditor` as `onEscapeKeyDown`.

**Protobuf and gRPC (M16)**
- `ContentKind::Cbor` is the last variant and `FlowDetail.error_kind` (R3, session format 8) the last field of `FlowDetail`; `grpc_status` is `FlowRow`'s last field. New kinds or detail fields go after them, with a `VERSION` bump.
- All decoding is in Rust: `Engine::decode_body_in` serves the inspector's `grpc`/`decoded` viewers and MCP's `get_flow_body` `format: "decoded"` alike. Don't decode protobuf, MessagePack or CBOR in the UI; the mock backend serves the engine's output (`<fixture>.<part>.decoded.json`).
- Decoded JSON is written by `io/tree.rs` in field order (never via serde_json's sorted `Map`) and travels as text, so 64-bit numbers stay exact. Notes on values are `Badge`s by JSON pointer.
- Schemas live compiled in `app.sqlite` (`proto_schemas`); mappings are the `proto_mappings` settings row. Any change goes through `Protos::reload`, which bumps the generation the decode cache keys on.
- `.proto` files compile with `protox` (no `protoc`), in the app and in pm-testkit's `build.rs`. Tests import `pm_testkit::grpc::SHOP_PROTO` with `ProtoImport::Sources`.
- Trailers-only gRPC responses carry `grpc-status` in the response headers and have no trailers; read the status with `io::grpc::status_code` on the trailers, then the headers.

**Network plumbing (M18)**
- Every upstream TCP connection goes through `proxy/dial.rs::dial` (DNS override → hop → loop guard); never call `TcpStream::connect` or a hyper-util `HttpConnector` elsewhere in `pm-core`. TLS and client certificates are layered on top in `mitm::Dial`. The trust probe and onboarding server never dial.
- The plain-HTTP pool is `Engine::pool()` (one per engine, `dial::Connector`); `rebuild_pool()` drops it when the proxy or network settings change. Its responses carry a `dial::PoolInfo` with the route and first-flow timing.
- Settings › Network is one `network` row (`NetworkSettings`), applied by `Engine::set_network_settings`. Passwords never stay in it: they go to the `SecretStore` and the row keeps `has_password`. Only the app sets `EngineConfig::use_keychain`; tests and the CLI use `<data dir>/secrets.json`, so tests never touch the keychain.
- `Via` (the route) is recorded with `FlowRecorder::via`, which keeps a reverse proxy's port. The SOCKS5 listener and CONNECT share `mitm::decide`; reverse proxies go through `forward` with `Forward { reverse_port, client_tls }`.
- A failed upstream request reads its connection task's error (`mitm::Died`) before explaining itself: with TLS 1.3 that is where `certificate_required` shows up.
- Integration tests are `crates/pm-core/tests/network.rs`, against `pm_testkit::{HttpProxy, Socks5Server, TlsUpstream::mtls}`. A "previous system proxy" test writes a `sysproxy-backup.json` owned by the test process into its own data dir; nothing restores it.

**WebSocket rules (M19)**
- Handshakes go through `proxy/exchange.rs` like any request (`Exchange.websocket: Some(ClientSide)`); on `101` the relay in `proxy/ws.rs` takes both connections over. Don't add a second handshake path.
- `Action::WsMessage` comes before `ServeFrom` (M20) and `Comment`, the last `Action` variant. Message rules are evaluated in `rules::eval::ws_message_phase` (unit matrix in `rules/tests.rs`, end to end in `crates/pm-core/tests/ws_rules.rs`); the relay re-matches when the rule set changes.
- Each direction is one ordered queue: a held message (delay, breakpoint, slow script) holds the ones behind it, and messages Proxymate sends (`Engine::ws_send`, through `proxy::ws::Registry`) join the same queue. Never write to a peer from anywhere but its direction's processor.
- Recorded messages carry `edited`, `injected` and `dropped`; an edited one keeps what arrived (`WsEntry.original`, the `originals` table, `Engine::websocket_original_in`). Every `WsLog::push` takes a `WsEntry`.
- A held message is a `BreakpointInfo` with `message` set; `Engine::resolve_breakpoint` refuses `Respond` and head edits for it.

**Offline mocking (M20)**
- `Action::ServeFrom { source, key, on_miss, pick }` comes just before `Comment`, the last `Action` variant, and `PresetKind::OpenApi` is the last preset. Sources are the mock library: `<data dir>/mocks/<uuid>.proxymate` copies, listed in `app.sqlite`'s `mock_sources` table (`mocks.rs`). Never point a rule at a user's original file.
- Rules compile with a `MockMap` beside the `ScriptMap` (`CompiledRule::compile(rule, scripts, mocks)`); `Engine::mock_map(rules)` opens and indexes (once, cached) the sources the enabled rules serve from. A disabled rule only needs its source to exist; a source a rule uses can't be removed.
- ServeFrom is terminal like Map Local: the first answer (Map Local, a mock script, a recording) wins (`RequestEffects::answered`). A hit is read with `MockIndex::response` on a blocking thread and goes through `respond` as `Answer::Upstream`, so response-phase actions run and an untouched body reaches the client as recorded (still content-encoded). A body stored decoded (HAR, Charles `text`) loses its `Content-Encoding`.
- The key is method, scheme, host and path (not the port), plus the query as sorted decoded pairs unless `key.query` is off, plus a hash of the decoded request body with `key.body` (which buffers the request body). A pass-through miss does nothing; 404/502 answers say what wasn't found.
- Charles: `.chlsj` and `.chlsx` open as documents (`DocumentKind::Charles`), told from HAR by contents (`io::charles::sniff`); a base64 body is the wire bytes (its encoding kept) unless it says `decoded`, a `text` body is decoded. Binary `.chls` is refused with `charles::BINARY_UNSUPPORTED`. The fixtures in `tests/fixtures/charles/` are hand-built from the documented format, not Charles 5 exports.
- OpenAPI: specs are their source text in `app.sqlite`'s `openapi_specs`, compiled into an `OpIndex` snapshot (`openapi.rs`, `serde-saphyr` for YAML). Only local `$ref`s are followed. Operations match method and path under a server base path; the host isn't compared.
- `op:` binds at parse time: `Engine::parse_query` (MCP, `find_flows`, `search_flows`) and each store's `set_operations` (view filters, via `CompiledFilter::compile_with`). An unbound `op:` matches nothing, so anything new that parses a query for flows must bind it. Views keep the snapshot they were filtered with.
- Generated mocks are `Map Local` rules saved disabled (`Engine::create_rules`, one change), with a regex path and the spec's one absolute host if its servers agree.

**Comment rules**
- `Action::Comment { text }` is the last `Action` variant. Its text is trimmed and checked at compile time (`CompiledAction.comment`, at most `workflow::COMMENT_MAX`). The request phase collects the comments of rules without a response matcher (before a `Block` returns), the response phase those with one; the exchange sends each as `FlowEvent::RuleComment`, which the store adds to the flow's comment after "; ". Never set a rule's comment with `FlowEvent::Comment`: that replaces the user's note.
- A comment isn't an applied rule: it doesn't set `edited` or appear in `FlowDetail.rules`.
- MCP `filter` descriptions come from `filter_dsl!` (`pm-mcp/src/tools/flows.rs`); a new DSL key goes in `FILTER_KEYS` (`store/query.rs`) and there, or `filter_doc_lists_every_key` fails.

**Workflow (M17)**
- The table's selection is flow ids (`lib/selection.ts`: ids, or a whole view with exceptions), never positions. Bulk actions take it as a `FlowSelection` (`currentSelection()` in `lib/actions.ts`); `Engine::selected_ids` resolves one in any store.
- `extra` is `FlowRow`'s last field (header column values); `label` and `comment` come before it. New row fields go before `extra`, with `decodeRow` and its test.
- Labels, comments and pins of the live store go through ingest; a document's are applied to its store directly. Session format 4 keeps them in `annotations`; a new annotation is a new column there, not a `FlowDetail` change.
- Header columns are global: `Engine::set_header_columns` bumps a generation, recorders send values for new flows (`Engine::header_cells`) and a background thread backfills the rest. Never write a value without the generation it belongs to.
- `remove_flows_in` frees segments by their oldest and newest writer (`SegmentStore::in_use_through`); every `append` must still name its flow.
- Copy as code renders a `RequestSpec` in `io/snippets.rs`; add a language there, in `CodeLanguage` (after the last variant), in `CODE_LANGUAGES` (`lib/flow.ts`) and a golden directory.
- `Engine::repeat` sends through `Engine::send`; anything that sends many requests should too, so rules, recording and `sent_by` stay consistent.

**Capture targets (M21)**
- M21 adds no `Action`, `PresetKind`, `ContentKind` or `FlowDetail` change: the session `VERSION` stayed 7 (R3 made it 8). `ServeFrom` was still the last `Action` variant (`Comment` came after it).
- Launchers live in `pm-platform/src/launcher.rs` behind `Launcher`; every command line goes through `command::Runner` (`spawn` for browsers, `run` for `open`), checked against the fake. **Tests never launch anything**; real launches need the user's OK.
- A launch owns `<data dir>/intercept/<uuid>/` (profile or script, plus a `pid` file); `capture.rs` removes it when the root pid exits, and the next start sweeps ones whose pid is gone. Terminals report their shell's pid through that file.
- Intercepted flows are named in `client.rs` by walking parent pids (`ProcessInfo::parent_pid`) to a tracked root: `<App> (Intercepted)`. `Engine::track_intercept` (hidden) tracks a pid without launching, for tests.
- The recording lists are checked in `Engine::begin_flow`: one `ArcSwapOption` load when both are empty, a non-allocating `GlobSet` match otherwise. An unrecorded flow has no recorder, exactly like a paused one, and is counted in `StorageStatus.unrecorded`. Hosts are normalized (`capture::normalize_host`) before they're stored; `*.x` also matches `x`.
- No Caching and Block Cookies are the built-in rules `builtin-no-cache` and `builtin-no-cookies` (`pm_model::NO_CACHE_RULE_ID`, `NO_COOKIES_RULE_ID`), made in `Rules::load` beside `builtin-throttle`, origin `User`, hidden from the Rules Editor like the throttle. The toolbar flips `enabled`.
- Auto-save runs from the publisher tick (`Engine::autosave_tick`) through `spawn_save`, writing `<name>.proxymate-partial` then renaming; rotation only deletes `Proxymate Autosave *.proxymate` files, oldest by modification time first.
