# Proxymate: HTTP(S) debugging proxy (Tauri 2 + Rust)

## Context
We're building a fast, modern desktop tool for debugging web traffic, in the same space as Charles Proxy and Proxyman. Requirements:
- HTTPS decryption, with a wizard for installing the certificate.
- One unified rules system that covers map local, rewrite, breakpoint, map remote, throttle and block.
- Colour-coded methods, statuses and URLs, in the style of Proxyman.
- A built-in MCP server so AI agents can inspect traffic and manage rules.
- macOS first, then Linux and Windows.

`/Users/cniekirk/Projects/proxymate` is empty, so this is a greenfield build. Installed toolchains: cargo/rustc, node and npm (no pnpm), Swift, and xcrun.

**What's in v1:**
- HTTP/1.1, HTTPS MITM, HTTP/2 and WebSockets.
- Mobile device setup wizards.
- HAR import/export, saved sessions, copy as cURL, replay and compose.

**What's out of v1:** HTTP/3, ws-over-h2 (RFC 8441), and a privileged helper. (JS scripting, first out of v1, is M14: see "Scripting (M14)".)

**MCP access:** agents get full access by default. A setting switches the server to read-only, and rules created by agents are tagged.

**Delivery:** this is a new GitHub repository. Each milestone below is built in its **own Claude session** and lands as **its own PR** (see "Delivery workflow").

Design cues from Proxyman's dashboard:
- Three panes: a sidebar grouped by Apps and Domains with counts, a dense monospace flow table, and a split Request | Response inspector.
- A row of filter chips: All/HTTP/HTTPS/WebSocket | JSON/Form/XML/JS/CSS/GraphQL/Document/Media/Other.
- Method and status pills, and a highlight on the selected row (Proxyman's is orange; ours is macOS blue).
- A toolbar status reading "Listening on IP:port", and a footer with Clear / Filter / Auto Select, the row count and throughput.

## Architecture

### Workspace layout
The Cargo workspace uses edition 2024, plus an npm workspace for the UI.
```
crates/pm-model     pure types (serde + specta + schemars): Flow*, Rule/Matcher/Action, FilterSpec, Settings
crates/pm-core      headless Engine: proxy/, tls/, store/, rules/ (compile, eval, local, breakpoints, app.sqlite), io/ (har, curl, session) — no Tauri dep
crates/pm-platform  OS traits + cfg impls: sysproxy, trust, procinfo, netif, simctl, adb (macos/ linux/ windows/)
crates/pm-mcp       rmcp server over Arc<Engine>: Streamable HTTP (axum) + stdio bridge
crates/pm-testkit   test upstreams (h1/h2 TLS, WS echo, SSE, slow/huge bodies), test CA; re-exports the synthetic flow generator (which lives in pm-core::synth so the app and CLI can seed data)
apps/desktop/src-tauri   thin Tauri adapters → Engine; also `proxymate mcp-stdio`, `--sysproxy-guardian`
apps/desktop/ui          Vite + React 19 + TS
apps/cli                 proxymate-cli serve|mcp-stdio|gen-flows|bench (headless, CI/agents)
```
- The Tauri commands and the MCP tools both call only the `Engine` facade, so the UI and agents always see the same behaviour: `start`, `set_recording`, `stores`, `rules`, `breakpoints`, `subscribe`, `send`.
- Dev profile: `[profile.dev.package."*"] opt-level = 2` so rustls and the decompressors are usable in dev builds. Release uses thin LTO.

### Data path (performance core)
1. **Per-request tasks.** Each request task emits small `FlowEvent`s (Started, ReqHead, RespHead, BodyProgress, Completed, Failed, WsMsg, ClientResolved) onto an mpsc channel. Body bytes never travel in these events.
2. **Ingest.** A single writer thread (`pm-ingest`) batches events (up to 5 ms per batch) and takes one write lock per batch. It updates the summary arena and the live views.
   - Flow ids are allocated and the Started event queued under one mutex, so events arrive in id order and the arena never reorders. Synthetic inserts use the same channel.
   - Ids are never reused. A clear empties the arena and moves its base id up; late events for cleared flows are ignored. Views index the arena by position.
3. **Publish.** A publisher ticks at 20 Hz and sends one `ViewDelta` per view over a `tauri::ipc::Channel`. Each delta carries totals, the appended count, the changed ids and a structural flag.
4. **UI fetch.** The UI pulls only the visible window with `get_rows(view, start, count)`, which returns compact tuples. Bodies come back as raw `ipc::Response` ArrayBuffers.

### Storage
- **Flow summaries.** `FlowSummary` (about 200 B) lives in an in-memory `Vec`. Hosts, apps and content types are interned.
- **Bodies.** Bodies of 8 KB or less are inlined. Larger bodies are stored still content-encoded in rotating segment files under `~/Library/Application Support/Proxymate/live/<uuid>/` (64 MB each since M10, so eviction frees disk in useful steps; 256 MB before). They are decoded only when viewed.
- **Flow detail.** `FlowDetail` is postcard-encoded into the segments, with a moka LRU in front of them. A flow still in flight is read from its live recorder instead.
- **Message logs (M6).** WebSocket messages and server-sent events live in per-flow logs beside the detail, not inside it, so a long-lived connection never makes its detail slow to load. They are paged through the engine (`websocket_messages`, `websocket_payload`, `sse_events`) with direction and text filters evaluated in Rust. WebSocket payloads are capped at 1 MB each; those over 1 KB spill to the segment files. SSE event data is capped at 1 MB per event.
- **Live directory lifetime.** Clear deletes the segment files and quit deletes the directory. Each live directory holds an exclusive file lock, so the next launch sweeps directories left behind by a crash or a signal.
- **Caps.**
  - Body capture is capped at 64 MB. Above that the recorded copy is truncated, but forwarding is never affected.
  - Flows are capped at 250k with ring eviction. Pinned flows are kept.
  - (M10) Limits are `StorageSettings` (`app.sqlite` row `storage`, Settings › Storage): flows kept (250,000; 0 = no cap), disk (4,096 MB; 0 = no cap) and WebSocket messages per connection (50,000). After every ingest batch the live store evicts the oldest flows past a limit, down to 95% of the flow cap (so eviction runs once per twentieth of the cap), keeping pinned flows and flows still in flight; the arena stays in id order, and a few kept old flows may sit before a gap (lookups count back from the newest flow, then binary-search). Views rebuild over the new positions; the sidebar counts drop the evicted flows.
  - (M10) Each segment file remembers the newest flow that wrote to it. An eviction through id N deletes every finished segment whose newest writer is at most N, except the segments kept flows (pinned or in flight, older than N) still use. The disk cap evicts through the newest writer of the oldest segments it must delete to get back under 90%.
  - (M10) A WebSocket log keeps its newest messages up to the limit; older ones are dropped but keep their indexes (`WsPage.dropped`).
  - (M10) A failed segment write (a full disk) is reported in `StorageStatus.error` and the footer until a write succeeds; forwarding never stops.
- **Sessions.** A saved session (`.proxymate`) is a single SQLite file. A `BlobBackend` trait lets the same store and view code read live segments, saved sessions and imported HARs.
  - (M14) The format is version 2: `FlowRules` (the last field of the postcard `FlowDetail`) gained `scripts`, its last field. Version 1 files still open: a v1 detail blob plus one `0x00` byte decodes as v2 (`tests/fixtures/session-v1.proxymate`). M14 also fixed postcard reading `RuleOrigin` (an internally tagged enum, which postcard can't deserialize): it now serializes by hand, the same bytes for postcard and the same tagged JSON, so flows a rule changed load from sessions and evicted live details again.
  - (M8) Tables: `meta` (format, version, saved time, app version), `flows` (the summary fields as columns, enums as their serde names; the `FlowDetail` as a postcard blob; the WebSocket message list and parsed SSE events as zstd'd postcard blobs), `bodies` (zstd, still content-encoded) and `messages` (WebSocket payloads, zstd). Flow ids are renumbered `0..n` in the order saved.
  - (M8) Saves run on a background thread with progress, into a temporary file in the target directory that replaces the target only once complete. HAR exports stream one entry per line the same way.
  - (M8) Every opened document is its own store (`StoreId`; the live capture is `LIVE_STORE` = 0): a `Store` for summaries and views, and a `SqliteBackend` for details, bodies and message logs (`SegmentBackend` is the live one). A document's view ids are `store << 24 | n`, so view calls need no store; flow calls take one (`Engine::*_in(store, …)`, and the Tauri commands' `store` argument). An imported HAR is converted into the same tables in an in-memory SQLite database.
  - (M8) `pmbody://localhost/<store>/<flow>/<part>` serves any store (`live` or a document's id).
- **App database.** `app.sqlite` (rusqlite, WAL) holds rules, settings, the SSL include/exclude lists, the LAN client allow-list and (M7) the MCP settings, bearer token and audit log (`mcp_audit` table).
  - The SSL lists (include, exclude, bypass, insecure upstream) still live in `<data_dir>/ssl.json` (M2).
  - (M9) The LAN client allow-list is a `devices` table (ip, nickname, allowed); LAN access on/off and the chosen interface are the `lan` settings row. Like the rules, they stay in memory for an engine without a data dir.

### Filtering at 100k+ rows (in Rust)
- **Views.** Each `View` holds `{CompiledFilter, SortSpec, indices: Vec<u32>}` and is maintained incrementally as flows arrive and change.
- **Refilter speed.** A full refilter is a linear scan of the summaries (rayon above 50k), with a target under 10 ms.
- **Deep filters.** Header and body predicates run as a cancellable background job that streams matching ids into the view.
  - (M10) The DSL's `header:`/`reqheader:`/`resheader:` (a substring of a `name: value` line) and `body:`/`reqbody:`/`resbody:` (a substring of the decoded body, its first 4 MB) are deep terms (`store/deep.rs`). The summary pass treats them as unknown; a view whose filter has any queues every finished flow that passes the rest as a candidate, and a `pm-deep-search` thread per view checks them 512 at a time on the rayon pool and adds the matches (appended in arrival order, else a resort). Flows still in flight are checked when they finish. A filter change or dropping the view cancels the job. `ViewDelta.deep`/`ViewInfo.deep` carry `{checked, total, matched, running}` for the footer.
  - (M10) MCP's `list_flows`/`wait_for_flow` evaluate the same terms inline, flow by flow, only until the page is full.
- **Filter DSL.** One handwritten parser (`store/query.rs`, M7) is shared by the search box and MCP. Example: `host:*.example.com method:POST status:4xx ct:json app:Safari dur>500 size>1mb -host:*.google.com body:token "text"`. (M10) `is:pinned`; the search box autocompletes keys, operators and known hosts, apps and content types (`ui/src/lib/dsl.ts`).

## Proxy engine (pm-core)

### Crates
| Purpose | Crate |
|---|---|
| Runtime | tokio |
| HTTP/1.1 + HTTP/2 | hyper 1.x + hyper-util (explicit http1/http2 builders, `preserve_header_case`) |
| TLS | rustls 0.23 + tokio-rustls with the `ring` provider |
| Upstream certificate checks | rustls-platform-verifier (uses the OS trust store, so corporate CAs work) |
| Pooled upstream client | hyper-util's pooled client for plain HTTP; must not use the system proxy, to avoid loops. Decrypted HTTPS doesn't pool: each client connection gets its own upstream TLS connection, opened during the handshake so its ALPN can be mirrored, and redialled if the upstream closes it (M2). hyper-rustls is not used |
| Cert generation | rcgen |
| Leaf cert cache | moka |
| WebSockets | tokio-tungstenite |
| Content decoding | flate2, brotli and zstd, run synchronously on stored bodies when the inspector asks, and on bodies buffered for a rewrite or breakpoint (M5; up to 32 MB, so no async-compression) |
| Rule matching | regex (with `size_limit`) and globset |
| App database | rusqlite (bundled SQLite, WAL) |
| Concurrency | arc-swap, parking_lot, rayon |
| Scripts (M14) | rquickjs `=0.14.0` (QuickJS-NG), without `parallel`, `futures`, `loader` or `bindgen`; `url` for WHATWG URLs |
| QR codes | qrcode |
| Network interfaces | if-addrs |

We build our own engine and use hudsucker only as a reading reference. Its handler model can't express editable breakpoints or ALPN mirroring.

### Connection pipeline
1. **Accept and sniff.**
   - Loopback clients and this machine's own addresses are always allowed. A new LAN IP raises an "Allow?" prompt (with a nickname field), and its connections are held for up to 30 s while the user decides, then refused (`403` for HTTP). Answers persist; pending prompts don't. An allowed device's flows are labelled `Nickname (10.0.0.23)` (or just the IP) in the Client column and the sidebar's Apps.
   - Then peek at the first bytes to choose a path: `CONNECT` goes to the tunnel, an absolute-form URI goes to the forward proxy, and an origin-form request addressed to us goes to the **onboarding server** (`/`, `/cert.pem`, `/cert.crt`, `/proxymate.mobileconfig`).
   - (M9) An absolute-form request for `proxy.mate` (any port) or for one of our own addresses on the listener's port also gets the onboarding server, which is what stops proxy loops. `CONNECT proxy.mate:443` is refused with `502`, so browsers that try https first fall back to the plain page. Onboarding requests are never recorded.
   - (M9) The `.mobileconfig` is an unsigned profile with one `com.apple.security.root` payload; its identifiers and UUIDs derive from the CA fingerprint, so reinstalling replaces it and a regenerated CA gets its own.
   - (M9) **LAN access** rebinds the listener from `127.0.0.1` to `0.0.0.0` on the same port (falling back to the old listener if that fails). The chosen interface (from `if-addrs`, Wi-Fi and Ethernet first) only decides the address the toolbar's "Listening on …", the setup URL and the QR codes show; the system proxy, the trust check and the terminal env keep using loopback.
2. **CONNECT.** Reply `200`, then decide between MITM and passthrough. The inputs are the SSL include/exclude globs, per-app exclusions and the auto-learned pinning bypass list.
   - The default is to MITM `*` except Apple, iCloud and OCSP hosts, plus a few others that pin.
   - Passthrough uses `copy_bidirectional` and records one tunnel flow.
3. **MITM handshake, upstream first.**
   - Read the client's ClientHello with `LazyConfigAcceptor` to get the SNI and ALPN.
   - Connect upstream offering the same ALPN, and see what upstream picks.
   - Finish the client handshake offering only that protocol. Three prebuilt `ServerConfig`s cover this: `[h2]`, `[http/1.1]` and `[]`.
   - If the upstream handshake fails, finish the client handshake anyway and answer every request with a readable 502 page; each request's flow fails with the reason and keeps the rejected chain for the TLS tab.
   - Plain HTTP inside a CONNECT is served as HTTP. Passthrough tunnels, and connection-level failures with no request, are recorded as one `CONNECT` flow.
   - The leaf chain sent to clients includes the CA. Upstreams are verified with `rustls-platform-verifier` plus any configured extra roots; a per-host "allow insecure upstream" list skips verification (signatures are still checked).
4. **Leaf certs.**
   - Each host gets an ECDSA P-256 leaf cert with SAN, EKU serverAuth, validity of 365 days or less (backdated 1 day) and a unique serial.
   - All leaves share one keypair, so issuing a cert takes about 50 µs.
   - The CA is ECDSA, with `CA:true` and `keyCertSign`. Its CN includes the machine name and date. The key file is mode 0600, and the user can regenerate the CA.
5. **Pinning detection.** If the client sends an `unknown_ca`, `bad_certificate` or `certificate_unknown` alert, closes during the handshake, or closes within 2 s of the handshake without sending any data, mark the flow `likely_pinned` and offer "Add to SSL bypass". The same signal appears when a client simply doesn't trust the CA yet, and the UI says so.
6. **WebSockets.**
   - Strip `Sec-WebSocket-Extensions` so frames stay uncompressed.
   - On `101`, wrap both upgraded connections and relay the frames, recording direction, opcode, size and time.
   - Payloads are capped at 1 MB.
   - Handled on every h1 path: absolute-form `ws://`, plain HTTP inside a CONNECT, and `wss://` inside the MITM (the upstream h1 connection is taken over by the upgrade; the next request redials).
   - The flow's scheme is `ws`/`wss`, its status pill reads "Active" while open, and the close (code, reason, which side) is recorded; a connection that drops without a close frame is recorded as `1006`.
7. **Streaming.** Bodies are tee'd by default, so SSE and LLM token streams stay live in the UI. A `text/event-stream` response is also parsed incrementally into events (id, event, data, retry), each timestamped as it completes.
8. **Port.** Listen on 9090. If it's taken, try 9091–9099 and show the port in use.
   - (M10) The port and the plain-HTTP pool (idle timeout, idle connections per host, connect timeout) are `ProxySettings` (`app.sqlite` row `proxy`, Settings › Proxy and Advanced); a change restarts the listener on the same interfaces and keeps the old one if the new port can't be bound. A bind failure says which app holds the port (`ProcessInfo::listener_pid`) and points at Settings.
9. **Proxied terminal.** "Copy terminal env" sets `HTTP(S)_PROXY`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and `CURL_CA_BUNDLE`. (M10) The last three replace a tool's roots, so they point at `<data dir>/ca/bundle.pem` (the OS roots from `rustls-native-certs`, then our CA), rewritten each time; Node's adds to its own and gets the CA alone.

### Known limitations (documented in the UI)
- The Raw tab is a reconstruction, not the exact wire bytes.
- TLS 1.0/1.1 upstreams fail, with a clear error.
- h2c is recorded but deferred.
- HTTPS in absolute form (`GET https://…` sent to the proxy) is refused with 501; clients must use CONNECT.
- WebSocket messages are reassembled and re-framed, so fragmentation on the wire may differ from what the peer sent. Pings are forwarded and also answered by the proxy, so a pinging side may see two pongs.
- (M19) A WebSocket message held by a delay or a breakpoint holds the ones behind it in its direction (up to 1,024 read ahead, then the peer is pushed back). Control frames (ping, pong, close) are never held or rewritten.
- A compressed (`Content-Encoding`) event stream is parsed once it is complete, without per-event times.
- The flow table's Status column shows `101` for a closed WebSocket; the close code and reason are in the WebSocket panel.
- Clear drops pinned flows too. (Until M17 a saved session didn't keep pins; format 4 keeps them with labels and comments.)
- A deep search reads each candidate's detail and decoded body (its first 4 MB) from disk, so on a large capture it takes seconds; rows stream in as it goes.
- HTTP/1.1 header names are forwarded with their original case, but recorded lower-cased (hyper doesn't expose its case map) and shown in Title-Case.
- LAN clients are identified by IP address: a device that gets a new address is asked about again, and renaming a device doesn't relabel its earlier flows. LAN access listens on IPv4 only.

## Rules engine
- **Data model.** A `Rule` has `{id, name, enabled, order, origin: User|Preset{preset}|Agent{client, session, created_at_ms}, matcher, actions[]}` (`pm-model/src/rules.rs`, serde + specta + schemars so the M7 MCP schemas are the same types).
- **Matcher.** Matches on methods, scheme, host, port, path, query and header KV matches (each can be negated), client app, and an optional response matcher (status ranges, content-type).
  - Patterns can be Exact, Glob or Regex, each with a case-insensitive flag. Exact and globs match the whole value; regexes are unanchored. In globs `*` crosses `/`, and a host glob `*.example.com` also matches `example.com`.
- **Actions:**
  - Headers: `SetHeader`, `AddHeader` (M13: appends a value, for repeated headers), `RemoveHeader`, `ReplaceHeaderValue`.
  - Query: `SetQuery`, `RemoveQuery`.
  - Body: `ReplaceBody`, `SetBody`, `JsonSet`, `JsonRemove` (by JSON pointer; `JsonSet` creates missing members, `-` appends; M13: `JsonSet.raw` is the value as exact JSON text, and both edit the bytes in place, see "M13"), `FormSet`, `FormRemove` (M13: form-urlencoded fields).
  - Status: `SetStatus`.
  - Routing: `MapLocal` (an inline body, a file, or a directory root mapped by path suffix: the whole path is tried, then with leading segments dropped; `..` never leaves the root) and `MapRemote`.
  - Flow control: `Breakpoint{request,response}`, `Delay`, `Throttle` (token bucket, kbps down/up plus latency), `Block` (a status code or a connection reset: h1 closes the connection, h2 resets the stream).
  - (M14) Scripts: `Script{script_id}`. It names a rewrite or mock script (see "Scripting (M14)").
  - (M19) WebSocket messages: `WsMessage{direction?, filter?, op}`, the last variant (see "M19").
  - Header, body and delay actions carry a `phase` (request or response).
- **Compilation.** Rules compile to a `CompiledRuleSet`, which is swapped in atomically with ArcSwap.
  - Every pattern becomes a byte regex (globs via globset) built with `RegexBuilder::size_limit` (256 KB compiled, 4096 characters of source). Errors name the rule, the field and the reason ("rule "x", host: invalid regex: unclosed group").
  - A host `RegexSet` prefilters candidate rules.
  - Every rule is compiled on save, enabled or not, so a broken rule is never stored.
  - Each flow snapshots the rule set when it starts. (M19) A WebSocket's messages are matched against the current rule set, again whenever it changes, so a rule added while a connection is open applies to its next message.
- **Persistence.** `app.sqlite` (rusqlite, bundled, WAL) in the data dir holds the rules (one JSON document per row, so new action fields need no migration) and the rule settings. An engine without a data dir keeps them in memory. The SSL lists stay in `ssl.json` for now; the device allow-list joined in M9.
- **Evaluation order** (`pm-core/src/rules/eval.rs`, applied on the wire by `pm-core/src/proxy/exchange.rs`):
  1. **Request phase.** All matching rules run in `order`; this is not first-match.
     - A `Block` in any matching rule stops the flow before any rewrite.
     - Header, query and body rewrites apply in sequence.
     - If several `MapRemote` rules match, the last one wins. It is applied before the breakpoint, so the breakpoint shows the final URL. Another HTTPS origin is reached over a new HTTP/1.1 TLS connection (there is no pooled HTTPS client).
     - Request delays add up; the last `Throttle` wins.
     - `MapLocal` is terminal: the first one wins, it skips the upstream (and the request breakpoint, since nothing is sent), but response-phase actions still run on its answer.
     - (M14) A `Script` action runs in place, in rule and action order, so it sees the edits before it: a rewrite script's `onRequest`, or a mock script's `onMock`. A mock answer is terminal like `MapLocal`: the first answer (Map Local or mock) wins, and later script actions in the request phase don't run. A script that throws, times out or returns something unusable is skipped and recorded in `FlowRules.scripts`; the flow goes on without it.
     - The request breakpoint fires **last**, so the user edits exactly what will be sent.
  2. **Response phase.** Gated by the response matcher (against the response as it arrived): rewrites run (including `onResponse` scripts, given the request as it was sent), then the response breakpoint, then response delays and the throttle's latency.
  3. **Recording.** Each flow records which rules applied (`FlowDetail.rules`: applied rules with their phase, `rewrite_skipped`, `breakpoint_timeout`, `mocked`) and sets `edited`. That drives the Edited column and the inspector badges. The recorded request and response are what was sent after rules, not the originals (keeping both is a follow-up).
  - (M19) A WebSocket handshake goes through the request phase and, on `101`, the response phase's header actions and delay; its messages run `WsMessage` actions and `onWsMessage` scripts (see "M19").
- **Breakpoints.**
  - (M10) The Breakpoint Editor edits bodies in CodeMirror, highlighted from the Content-Type.
  - `BreakpointManager` keeps a `DashMap<BpId, Pending{snapshot, oneshot::Sender<Resolution>}>`.
  - A resolution is `Continue{edits}`, `Abort` or `Respond`. Edits can change the method, URL (another origin is dialled directly), status, headers and body.
  - The timeout comes from the rule settings: 300 s by default, 0 waits forever. On timeout the flow continues unmodified and is marked `breakpoint_timeout`.
  - The UI and MCP can both resolve a breakpoint. The first to resolve wins; later ones get "already resolved".
  - Bodies up to 16 MB can be edited; above that only the head. A paused flow shows the violet Paused pill; other h2 streams on the connection keep flowing. If the client goes away while paused, the flow fails.
  - The queue is pushed to the UI on the publisher tick whenever it changes.
- **Body policy.** By default bodies stream. If any body action, script or breakpoint applies, the body is buffered up to 32 MB, decoded, transformed, and re-sent identity-encoded with a fresh Content-Length (h1 chunking is dropped). Decoding is synchronous on the stored bytes (no async-compression).
  - (M14) A buffered body that nothing changed (a script that only read it, a breakpoint continued without editing it, a body action that matched nothing) is forwarded as the original wire bytes, still content-encoded. An event stream is never buffered for a script.
  - A body over the cap is forwarded untouched (what was read, then the rest of the stream) and the flow is marked `rewrite_skipped`; so is a body that can't be decoded.
- **Presets.** "Map Local…" and "Breakpoint…" in the flow context menu create pre-filled rules (`Engine::draft_rule_in`, matching the flow's method, host and path), on live flows and (M13) saved sessions and HARs. "Rewrite…" opens the Rewrite dialog (M13); its preset drafts no actions. The rules editor has a "Test against selected flow" dry run (`Engine::test_rule_in`, any store), computed in Rust with the proxy's own evaluator.
- **Throttle.** The toolbar's network conditions are the built-in rule `builtin-throttle`: it matches everything, sits ahead of every other rule, is disabled by default and can't be deleted.

## Scripting (M14)
JavaScript on an embedded QuickJS (rquickjs 0.14, QuickJS-NG). Scripts get no network, file access, `require`, `import` of other modules or timers. `crates/pm-core/src/scripts/proxymate.d.ts` is the API's source of truth; `prelude.js` (the JS half) and `native.rs` (the Rust built-ins) implement it.

- **Kinds.** A `Script` (`pm-model/src/scripts.rs`) is `{id, name, kind: rewrite|mock|view, enabled, source, origin: RuleOrigin, view: Option<ViewConfig{matcher, parts, kinds}>, updated_at_ms}`.
  - Rewrite scripts export `onRequest(context, url, request)` and/or `onResponse(context, url, request, response)`; mock scripts `onMock(context, url, request)`, returning `{status, headers, body}` (or a string, bytes, any object as JSON, or `null` to pass). Both run only from a rule's `Action::Script{script_id}`, so one script serves many rules.
  - View scripts export `onView(context, flow)`, returning `[{title, kind: json|text|xml|html|hex|image, body}]`, and run when the inspector shows a body their `ViewConfig` matches.
- **Runtime (`pm-core/src/scripts/`).**
  - `ScriptHost` runs N = clamp(cores/2, 1, 4) worker threads (8 MB stacks), plus one for dry runs, started on first use. Each owns a `Runtime` and `Context` (a `Runtime` isn't `Send` without `parallel`). Jobs are sharded by script id with a bounded queue of 256 each (a full queue is a script error); answers come back over a tokio oneshot.
  - Each job carries an absolute deadline set when it is queued: a job still waiting when it passes fails without running, and a running one is stopped by the interrupt handler. 1000 ms for traffic and mock scripts, 3000 ms for views (`ScriptSettings`, the `scripts` settings row).
  - 256 MB of QuickJS heap and 1 MB of stack per worker. The context (and its runtime) is rebuilt after an interrupt, an out-of-memory error or a stack overflow, and after 64 modules (QuickJS never unloads one). Async entry points are supported by running the job queue until the promise settles or the deadline passes.
  - Each script is evaluated once per worker as an ES module and its namespace cached by source hash. There is no bytecode cache (loading bytecode is `unsafe` and unsound for untrusted input). `context.shared` is kept per script and source: saving a script, or a context rebuild, starts it over. Dry runs use their own worker and a fresh module each time, so they never touch traffic scripts' `shared`.
  - Saving a script evaluates it once (with the traffic timeout) to check it parses and exports its kind's entry point; a broken script is never stored.
- **Messages.** `method, url, scheme, host, port, path, query, headers, body, text, rawBody, bodyTooLarge` (and `status`). Header and query keys are lower-case (query keys as sent), a repeated name is a `string[]`. `body` is parsed lazily: JSON (with `JSON.parse` source access, integers beyond 2^53 as BigInt; written back with `JSON.rawJSON`), a form object, text, or `null` for binary. Dirty detection keeps an untouched message byte-identical: headers and query are compared with what came in, a read `body` with its canonical form when first read, `rawBody` byte for byte. Bodies over 16 MB, and event streams, reach scripts as `null` with `bodyTooLarge`.
- **Built-ins.** `atob`/`btoa` (QuickJS-NG's own), `TextEncoder`/`TextDecoder` (UTF-8, Rust-backed), `URL` (the `url` crate) and `URLSearchParams` (JS), `crypto.sha1/sha256/sha512/hmac/randomUUID` (`ring`, `uuid`), `Proxymate.base64Decode/base64DecodeText/base64DecodeJson/base64Encode` (standard and URL-safe, padded or not), `Proxymate.decompress` (the inspector's decoder), `Proxymate.parseJson/stringifyJson` (BigInt-safe) and `console.*` (the script console). `scripts/api_surface.txt` lists them; a Rust test checks it against the runtime's globals and a vitest checks it against the `.d.ts` and the editor's completions.
- **Rules.** `CompiledRuleSet::compile(rules, scripts)` resolves each `Action::Script` to the `CompiledScript` (source, exports) as it is then; each flow keeps the snapshot it started with, so editing a script never changes a flow halfway. A missing script or a view script in a rule is a compile error; deleting a script some rule runs is refused, naming the rules; a disabled script does nothing. `touches_body` is true for a script in each phase it exports. Script problems are `ScriptIssue`s in `FlowRules.scripts` (session format 2) and show as badges in the inspector.
- **View scripts.** `Engine::run_view_scripts(store, flow, part)` matches each enabled view script on the summary and detail first (its matcher through the rules compiler, plus parts and content kinds) and only then reads the bodies. It works on the live capture, saved sessions and HARs. Results for finished flows are cached in moka (64 MB, weighed by bytes) by (store, flow, part, script id, source hash); flow and store ids are never reused, so no epoch is needed. Each view is capped at 8 MB.
- **Dry runs and console.** `Engine::test_script(store, flow, {script_id | source, kind, part})` runs a saved or unsaved script against a recorded flow in any store: before and after for rewrites, the mock's answer, or the views, plus what it logged. The script console keeps the newest 2,000 entries; subscribers are told the newest sequence number on the publisher tick and fetch entries with `script_console(since, limit)`.
- **Files.** Scripts are rows of a `scripts(id, json)` table in `app.sqlite`. Export and import are `.js` files with a `/* proxymate: {"name", "kind", "enabled", "view"} */` header; a file without one imports with its kind from its exports and its name from the file name.
- **UI.**
  - The Scripts window (`ui/src/components/Scripts/`; toolbar, ⌘K, File › Scripts… ⌘⇧J): scripts grouped by kind with enable switches and agent badges; a CodeMirror JavaScript editor with completions from a hand-kept map (`lib/scripts.ts`, checked against the `.d.ts`); "Where it runs" (view scripts: a `MatcherBuilder` plus part and kind toggles) or "Used by rules" with "Create rule for selected flow"; Test (before/after, the mock answer, or the views) and Console tabs; Copy type definitions, Export, Import. Templates (`ui/src/lib/script-templates/*.js`, run by a Rust test too): decode a base64 JSON field, decode a JWT, add a header, modify a JSON field, mock JSON.
  - `BodyViewer` gets a "Scripts" group in its viewer dropdown and shows the first script view unless the user picked a built-in viewer (`pickedScriptView`/`scriptViewsOff` in `store/ui.ts`). Views render through the existing viewers; a script's error shows as a note. Script views also show for bodiless messages (a JWT in a header).
  - The flow menu has "Script ›" (new view, rewrite or mock script for that flow); `ActionListEditor` has a "Run script" action with a script picker.
- **Risks.** C build of QuickJS on every target (prebuilt bindings exist for macOS x86_64/aarch64, Linux and Windows MSVC; whichever of M12 and M14 lands second shows it building on Windows CI); CPU-heavy scripts slowing flows (timeouts, queue bounds, and a flow never waits longer than the timeout); agent-written code running in-process (no I/O, read-only mode hides every tool that saves or runs agent code, agent tagging).

## macOS integration (pm-platform)
- **CA trust.**
  - Install the CA into user-domain trust in the login keychain with `SecTrustSettingsSetTrustSettings` via security-framework (fallback: `security add-trusted-cert -r trustRoot -k login.keychain-db`). macOS shows its own password/Touch ID sheet, and no sudo is needed.
  - The wizard verifies trust with a real handshake: a `rustls-platform-verifier` client connects to our own MITM for `verify.proxymate.test`.
  - Trust is re-checked at launch, and the toolbar warns if it's missing.
  - Uninstall removes the trust setting and deletes the cert.
  - For Firefox, the wizard explains `security.enterprise_roots.enabled`.
  - (M4) The probe host `verify.proxymate.test` is answered by the proxy itself: no upstream dial, no flow. An empty user-domain trust array ("always trust") counts as trusted; security-framework's own check misreads it.
- **System proxy.**
  - Use `networksetup` per active service (an enabled service with an IPv4/IPv6 address) for web, secure web and the bypass domains `localhost 127.0.0.1 *.local 169.254/16`.
  - **Spike result (M4, macOS 27.0 26A428):** `networksetup` writes without any prompt for an admin user (it is Apple-entitled for `system.services.systemconfiguration.network`), about 20-35 ms a call. The `SCPreferencesCreateWithAuthorization` fallback is not needed and was not built. Independent reads and writes run concurrently (checked safe: no lost updates).
- **Restoring the system proxy.**
  - Before changing anything, write a backup file containing the pid, host and port.
  - Restore on quit, on SIGINT/SIGTERM and from the panic hook.
  - A detached **guardian process** (`proxymate --sysproxy-guardian <pid> --data-dir <dir>`, in its own process group) waits on kqueue `NOTE_EXIT`, so a SIGKILL or crash still restores. Measured: 0.26 s from SIGKILL to `scutil --proxy` showing the proxy off.
  - At startup, a stale backup whose pid is dead also triggers a restore.
  - A service is only restored if its setting still points at us.
- **Client app detection.**
  - For loopback peers, map the peer port to a pid with libproc (`proc_pidinfo` FD scan, cached, coalesced, and run in spawn_blocking off the hot path).
  - From the pid, find the `.app` bundle to get the name and bundle id. Helper processes map to their parent app.
  - For XPC services and framework processes (Safari's network process lives in WebKit.framework) the pid macOS holds responsible (`responsibility_get_pid_responsible_for_pid`, looked up with dlsym) names the app. Plain executables (curl, node) show their own name.
  - Icons come from `NSWorkspace.iconForFile` via objc2-app-kit and are served through a `pmicon://` URI scheme. They follow the user's icon style (clear or tinted icons stay clear or tinted).
  - Per-app SSL exclusions (`SslSettings.exclude_apps`, by bundle id or tool name): a CONNECT waits up to 500 ms for its client app when any app is excluded.
  - The sidebar's Domains group hosts under a registrable domain by a small heuristic (two labels, three under `co.uk`-style suffixes), not the Public Suffix List.
  - Remote devices show their IP plus a nickname from the allow-list.
- **iOS Simulator.**
  - `xcrun simctl list devices booted -j`, then `xcrun simctl keychain <udid> add-root-cert`. "Install to all booted" is one click.
  - The simulator uses the Mac's system proxy, so no extra proxy setup is needed. (M9) The wizard checks that the system proxy is on and offers to turn it on.
  - (M9) Simulator processes are recognised by their `CoreSimulator` paths and named `<app> (Simulator)`; the wizard's live check waits for the first decrypted HTTPS flow from one. The Android emulator's traffic arrives from its qemu process and shows as `Android Emulator`.
- **Android (M9).**
  - Phones are set up by hand (Wi-Fi proxy, the CA as a user certificate, a debug-only `network_security_config.xml` for the user's own apps); the wizard shows the steps and a QR code of the setup page.
  - Emulators are set up over `adb` (from the SDK's usual places, then `PATH`): `settings put global http_proxy 10.0.2.2:<port>`, cleared with `:0` on quit, and `adb push` of the DER to `/sdcard/Download/` for the user to install.
- **Commands (M9).** `simctl` and `adb` run behind a `command::Runner` trait in `pm-platform`, so their command lines and output parsing are tested with a fake runner.
- **Linux and Windows equivalents (M12).**
  - Linux: `pkexec update-ca-certificates`/`update-ca-trust` plus NSS `certutil`; proxy via gsettings or kwriteconfig; app detection via netlink sock_diag and `/proc`.
  - Windows: the CurrentUser ROOT store; proxy via the HKCU Internet Settings registry key and `InternetSetOption`; app detection via `GetExtendedTcpTable`; the guardian uses `WaitForSingleObject`.

## MCP server (pm-mcp, rmcp)
- **Transport.**
  - Streamable HTTP on `127.0.0.1:9095/mcp` (rmcp `=3.6.0`, mounted in axum). `PROXYMATE_MCP_PORT` overrides the port; a taken port is retried every 5 s and the reason shown in Settings.
  - Each install gets its own bearer token (32 random bytes, in `app.sqlite`; regenerable). rmcp checks `Host` (loopback names only) and `Origin` (none, or a loopback page such as the Inspector's), which blocks DNS rebinding.
  - (M7) Only the protocol versions that still have `initialize` and sessions (up to 2025-11-25) are offered: the session carries the client's name, `tools/list_changed` and resource subscriptions. The 2026-07-28 stateless lifecycle is a follow-up.
  - (M7) A session idle for 5 minutes is closed (one-shot clients rarely send `DELETE`); the client re-initializes on its next request. The toolbar counts open sessions.
  - Settings has a copy button for `claude mcp add --transport http proxymate … --header "Authorization: Bearer …"` and for the `mcpServers` JSON.
- **Stdio bridge.** `proxymate mcp-stdio` (and `proxymate-cli mcp-stdio`) reads the token from `app.sqlite` in the app's data dir and relays JSON-RPC to the HTTP endpoint (rmcp's stdio transport on one side, its HTTP client transport on the other). If nothing listens it runs `open -g -b dev.proxymate.desktop` once and retries for 10 s.
- **Headless.** `proxymate-cli serve --mcp[=PORT]` runs everything without the UI, for CI.
- **Read-only mode.** The mutating tools disappear from `tools/list`, every session gets `tools/list_changed`, and a mutating call is refused (and logged as refused).
  - Every tool carries `readOnlyHint`/`destructiveHint` annotations.
  - Every call goes into the audit log (`app.sqlite`, newest 10,000 kept) with the client name from `initialize`, the arguments (cut at 4 KB), the outcome and the time taken.
- **Tool schemas.** Built from the parameter structs' schemars derives, which embed the `pm-model` rule and breakpoint types; nullable types and bare `true` schemas are rewritten so stricter clients accept them.
- **Filter DSL (M7).** `pm-core/src/store/query.rs`: `host: path: method: scheme: status: ct: app: state: is:edited dur size reqsize id`, `-` negation, quoted phrases, bare words search the URL. MCP parses strictly (unknown keys are errors); the search box parses leniently (a term that doesn't parse is text). Autocomplete is M10.
- **Read tools:**
  - `get_status`, `list_flows{filter?, since_id?, limit, cursor, sort}`.
  - `search_flows{query, regex?, scopes, filter?}`: URL, headers and decoded bodies (first 1 MB), newest first, 2,000 flows per call, with snippets.
  - `get_flow{id, include}`, `get_flow_body{id, part, decode, format, json_pointer?, offset, max_bytes}` (64 KB by default, 1 MB at most).
  - `get_websocket_messages`, `wait_for_flow{filter, timeout_ms, after_id, include_active}`.
  - `list_rules`, `get_rule`, `list_breakpoints`, `get_breakpoint`.
  - `export_curl{id}`, `export_har{ids? | filter, limit}` (inline JSON, up to 4 MB; bigger exports go to a file from the app).
- **Script tools (M14):** `list_scripts`, `get_script`, `get_flow_views` and `get_script_console` read; `create_script` (Agent origin), `update_script`, `set_script_enabled`, `delete_script`, `delete_agent_scripts` and `test_script` (it runs agent code, so read-only mode hides it too) write. Resources `proxymate://scripts` (updated on change) and `proxymate://scripts/api` (`proxymate.d.ts`).
- **Write tools:**
  - `create_rule`, `update_rule`, `set_rule_enabled`, `delete_rule`, `delete_agent_rules`. Rules agents create get `origin: Agent{client, session, created_at_ms}`.
  - Sugar tools: `create_map_local`, `create_breakpoint`, `create_rewrite`, targeting a URL pattern (`https://api.example.com/v1/*`) or a recorded flow (`flow_id`).
  - `resolve_breakpoint{id, resolution}` (M19: a held WebSocket message is continued with `body`/`body_base64` edits or aborted, which drops it), `set_recording`, `clear_flows`.
  - (M19) `send_websocket_message{id, direction, payload, binary}` and `close_websocket{id}`; `get_websocket_messages` reports each message's `edited`, `injected` and `dropped` flags and an edited one's `original`.
  - `replay_flow{id, modify?, apply_rules, wait}`, `send_request`. Both wrap `Engine::send(RequestSpec, SendOpts) -> FlowId` (M8): `request_spec(store, id, editable)` gives the recorded request to modify, `SendOpts{apply_rules, wait, origin: User | Agent{client}}` the rest; `har_json` and `curl_command` back `export_har` and `export_curl`.
- **Resources:**
  - `proxymate://status`, `proxymate://rules` (updated when rules change).
  - `proxymate://breakpoints`, which is subscribable and notifies on every hit (hits within one 50 ms publisher tick share a notification).
  - Templates: `proxymate://flows/{id}`, `…/request/body`, `…/response/body`, `…/ws`.
- **Prompts.** `investigate_failures` and `mock_endpoint`.
- **In the UI.** Agent rules show an "AI · <client>" badge, and there's a "Remove all agent rules" button. The toolbar shows an MCP indicator with the number of connected clients. Settings › MCP has the enable and read-only switches, the copy buttons, token regeneration, connected clients and the audit log. The engine pushes rule changes (`subscribe_rules`), so the Rules Editor follows edits agents make.
- **Engine.** The breakpoint queue, the rule list and the MCP status each push to any number of subscribers (`notify.rs`), so the UI and every MCP session can watch.

## UI (apps/desktop/ui)

### Stack
- React 19, TS strict and Vite.
- `@tauri-apps/cli` as an npm devDependency.
- tauri-specta generates both the TS types and the command wrappers. Fallback: ts-rs.
- **State and data:** Zustand for UI state and TanStack Query for detail, bodies and rules.
- **Table:** TanStack Virtual over a hand-built div table with fixed 22 px rows, plus a non-reactive row cache.
- **Body viewers:** CodeMirror 6 for bodies and Raw, and a custom virtualised JSON tree that parses in a Web Worker.
- **Design system: shadcn/ui** (Radix primitives + Tailwind v4 + lucide icons). The components are copied into `ui/src/components/ui/`, so we own them and can restyle them freely.
  - **Why shadcn:** it fits the stack already chosen, it's themed entirely through CSS variables (which matches the colour-token plan), and its command palette, context menu, resizable panels, tabs, dialogs, sheets and tooltips cover most of the chrome.
  - **Customisations for a native, Proxyman-like density:**
    - A compact scale: 12–13 px text, 24–28 px controls, tighter radii.
    - The system font stack (`-apple-system`/SF Pro), and SF Mono/JetBrains Mono for data.
    - A `ui/src/styles/tokens.css` that defines both the shadcn variables (`--background`, `--primary`, …) and the app tokens (`--m-*`, `--s-*`, `--url-*`).
  - **Building blocks:**
    - `Resizable` (react-resizable-panels) for the three-pane and request/response splits.
    - `Command` (cmdk) for a ⌘K palette and the DSL autocomplete.
    - `ContextMenu` for row actions, `Sheet`/`Dialog` for the wizards, and `sonner` for toasts.
  - **Not shadcn:** the flow table is the custom virtualised div table, not shadcn's DataTable, which is TanStack Table based and too heavy for 100k rows. The JSON tree, hex view and CodeMirror viewers are also custom.
  - **Native macOS chrome:**
    - Tauri `titleBarStyle: "Overlay"` with inset traffic lights and the toolbar drawn in the title bar area.
    - A sidebar vibrancy effect via the `window-vibrancy` crate (`NSVisualEffectMaterial::Sidebar`).
    - On Linux and Windows the sidebar uses a solid fallback colour.

### Layout
- **Toolbar.** Record, Compose, Clear, "Listening on …" (click it to open device setup), Throttle, the breakpoint queue badge, the system proxy toggle, cert status, MCP status and Settings.
- **Sidebar.**
  - Favorites, Pinned and Saved sessions.
  - All, split into Apps (icon and count) and a Domains tree. The counts are aggregated in Rust.
  - A filter box.
- **Filter row.** The chips plus a DSL search box with autocomplete.
- **FlowTable.**
  - Columns: #, state dot, colourised URL, Client, Method pill, Status pill, Time, Duration, Req size, Resp size, Proto (`h2`/`1.1`), SSL (decrypted, passthrough or likely pinned), Edited, Tools.
  - Columns can be resized, reordered, hidden and sorted.
  - The context menu has copy URL/cURL, Replay, Edit & Replay, Pin, Map Local…, Breakpoint…, Rewrite…, Export HAR and Add to SSL bypass. (M15) Its first item copies the cell under the cursor ("Copy Status 200"); a Copy submenu has the host, path, headers, decoded bodies and raw messages; Filter by narrows the search to the flow's host, method, status or app. The row's "…" opens the same menu.
- **Detail pane.**
  - A URL bar with method and status pills.
  - Request | Response split, with tabs Header, Query, Cookies, Body, Raw, JSON, Form, Hex, Preview, Timing and TLS. Timing and TLS sit in the Response pane; the URL bar shows the HTTP version.
  - A WebSocket frame panel and an SSE event list.
- **Footer.** Clear, Filter, Auto Select, "n/m rows selected", ↑↓ throughput and deep-search progress.
  - (M10) Throughput is the rate of change of the body bytes the store has seen (`StorageStatus.sent_bytes`/`received_bytes`, pushed with the storage status); a disk-full write error shows as "Not recording bodies".
- **Keyboard (M10).** `ui/src/lib/commands.ts` is the one list of commands: the ⌘K palette (shadcn `Command`) lists them all, and the ones with a key run from `hooks/useShortcuts.ts`: ⌘R replay, ⌘⌫ clear (not while typing), ⌘E record, ⌘F search flows (in a body viewer CodeMirror's own search), ⌘L filter apps and domains, ⌘, settings. The table takes ↑/↓, PgUp/PgDn, Home/End and Enter (to the inspector); Escape in the inspector returns. Every focusable element has a visible ring.
- **Columns (M10).** Resize by the header's edges (double-click resets), reorder by dragging headers, hide from the header's context menu; widths, order, visibility and the sort persist in the webview's local storage (`store/prefs.ts`).
- **Replay and documents (M8).**
  - `Engine::send` runs the request through the same `proxy/exchange.rs` pipeline as proxied traffic (rules on or off), from a client that is Proxymate itself: plain HTTP on the pooled client, HTTPS over a one-off HTTP/1.1 TLS connection (no pooled HTTPS client). The flow is recorded even while recording is paused, its summary carries `sent_by` (`FlowRow`'s last field), and the URL column shows a "Replay" (or "AI") badge.
  - Replay sends the recorded bytes as they were (still content-encoded); Edit & Replay opens Compose with the body decoded and `Content-Encoding` dropped. Host, Content-Length and hop-by-hop headers are derived when sending. WebSocket flows can't be sent yet.
  - Copy as cURL (`io/curl.rs`) single-quotes everything, turns an `Accept-Encoding` curl can decode into `--compressed`, sends text bodies with `--data-raw` and binary or content-encoded ones with `--data-binary @-` from embedded base64 (or a file the user saves first, above 256 KB).
  - HAR 1.2 (`io/har.rs`, our own serde types) reads Chrome's `_webSocketMessages`, `_resourceType`, `_error`, base64 bodies and h2 pseudo-headers (dropped). Imported bodies are the decoded text the HAR holds, so they are stored without a content-encoding.
  - Opened documents show as tabs above the filter row and in the sidebar's Saved section. File → New Request…, Open…, Import HAR…, Save Session… (the active tab, all flows) and Export HAR… (what the table shows); the flow menu's Export HAR… exports that flow.
- **Settings (M10).** Proxy (port, LAN access and interface, system proxy, terminal env), SSL Proxying (decrypt, never decrypt, bypass and insecure-upstream lists, apps never decrypted), Clients (the LAN allow-list: allow, deny, rename, forget), Storage (usage and limits), Appearance (theme, reset columns), Advanced (breakpoint timeout, upstream pool) and MCP. A closed "Allow?" prompt leaves the device pending with a "device waiting" button in the toolbar; the prompt counts down to when held connections are refused (`DeviceClient.held_until_ms`).
- **Errors (M10).** A failed flow's inspector leads with what went wrong and the fixes that apply (`ui/src/lib/errors.ts`): the certificate wizard and SSL bypass for a client refusing our certificate, insecure upstream for an untrusted server certificate, SSL bypass for old TLS, the timeouts for timeouts. Regenerating the CA offers to remove the old CA's keychain trust.
- **Context menus (M15).** Everything that shows data has a right-click menu with the copy forms you'd want plus the actions its buttons already offer.
  - Name/value tables (headers, query, form, multipart part headers, TLS, certificates, SSE fields) copy a row's name, value or line (`Name: value`, URL-encoded `k=v`), the whole table or the table as JSON; header rows add cookies, the `Authorization` token and "Filter flows with this header" (`reqheader:`/`resheader:`). The URL bar copies the URL and its parts, method, status and cURL. Code views copy the selection (CodeMirror's own, which can span lines that aren't rendered) or everything shown. Every body viewer's menu ends with Copy body, Copy body as Base64 and Save body; the Body tab also has a Copy button. Hex rows, WebSocket and SSE messages (payloads fetched whole), timing, rule badges, failure banners, the toolbar address, the CA, document tabs, sidebar rows, Settings, devices, rules, scripts, console lines, paused flows and the Rewrite preview have their own.
  - Copies go through `ui/src/lib/clipboard.ts` (a short "Copied …" toast; a failure explains itself). Texts that are cut for display (dry runs, test results) copy whole. List menus are one menu per list: rows carry `data-menu-key` and `components/menus/useMenuTarget` resolves the row under the cursor.
  - Our menus replace the WebView's, which would have offered Copy, so menus on selectable text start with "Copy ⌘C" for the selection. Right-clicks no menu of ours handles keep the native menu only in text fields, editors and over selected text; elsewhere a release build shows nothing (no "Reload"). Debug builds keep the native menu for Inspect Element. Iframes (HTML preview, PDF) and native media controls keep theirs.
- **Windows.** Compose, Rules Editor (drag to reorder, MatcherBuilder, ActionListEditor, dry run), Breakpoint Editor (queue plus editable message with Execute/Abort/Respond), Cert Wizard, Device Setup Wizard (Simulator / iPhone / Android / Android emulator, with QR codes and a live "waiting for device…" state), Settings and Session Manager.

### Body viewers and previews
Every body can be viewed as **pretty, syntax-highlighted text**, as a **structured view** or as a **visual preview**, with Raw and Hex always available as fallbacks.

- **Content detection (in Rust, `pm-core/src/io/content.rs`).**
  - Detection uses the Content-Type header first, then magic-byte sniffing (PNG, JPEG, GIF, WebP, AVIF, ICO, PDF, gzip), then a text heuristic (JSON or XML by first non-space byte).
  - The result is a `ContentKind` enum: Json, Xml, Html, FormUrlEncoded, Multipart, GraphQL, Javascript, Css, Text, Image, Svg, Video, Audio, Font, Pdf, Binary, and (M16) Grpc, Protobuf, MessagePack, Cbor. The M16 kinds have no signature, so their header beats magic-byte sniffing (a gRPC frame's length can read like an ICO header).
    - `ContentKind` stays fieldless (it is `Copy` and a plain string in `FlowRow`); the format within a kind (PNG vs WebP, MP4 vs WebM) is `BodyInfo.mime`, next to the per-body `BodyInfo.kind` (M3).
    - A binary signature beats any header, so mislabelled bodies follow their bytes. Content-encoded bodies are sniffed after decoding their first 16 KB.
  - The kind is stored in the summary, so it drives the filter chips too. The flow's kind is the request's for GraphQL operations and form posts, otherwise the response's, falling back to the request's.
  - GraphQL requests (`{query, variables, operationName}`, batches, persisted queries, GET `?query=`) are recognised in Rust, and the operation name is stored in the summary (`FlowRow`'s last field).
- **Decoding.** Bodies are decoded (gzip/br/deflate/zstd) on demand. Each inspector shows a "Decoded from br · 12.4 KB → 88.1 KB" note.
- **Viewer registry (`ui/src/components/BodyViewer/`).** One component per kind. Each pane auto-selects the best viewer and offers a dropdown to override it; the choice sticks per content kind. The viewers live in the Body tab's dropdown rather than as separate JSON/Form/Hex/Preview tabs.
  - **JSON:**
    - A Pretty mode (CodeMirror 6 + lang-json with line numbers, folding, bracket matching and in-body search). Pretty re-indents the text without parsing it into values, so number literals (`1.50`, 64-bit ids) are kept exactly.
    - A **Tree** mode: a virtualised, collapsible tree showing type-coloured values, child counts, and right-click copy for the value, key path and JSON pointer.
    - A JSONPath/pointer filter box.
    - Parsing for large bodies happens in a Web Worker.
  - **XML / HTML:** pretty-printed with highlighting (lang-xml / lang-html). HTML also gets a **Preview** tab: a sandboxed `<iframe sandbox>` with scripts disabled and a CSP that blocks network loads.
    - The previewed document is also stripped of every remote URL (DOMParser) before rendering, and the iframe carries the CSP as its `csp` attribute: Chromium's preload scanner fetches `<link>`/`<img>` URLs before a meta CSP applies, which matters for WebView2 (M12).
  - **Form URL-encoded:** a decoded key/value table (repeated keys kept in order), with a toggle to switch to raw.
  - **Multipart:** a list of parts showing each part's headers. Each part opens in the right sub-viewer, and file parts that are images render inline.
  - **GraphQL:** detects `{query, variables, operationName}`. The query is shown highlighted next to a JSON viewer for the variables. The operation name appears in the URL column. The highlighting is a small CodeMirror stream mode rather than lang-graphql, which would pull in the `graphql` package.
  - **JS / CSS:** shown highlighted, with an optional "Beautify" toggle (prettier standalone, run lazily in a worker).
  - **Images:**
    - Handles PNG, JPEG, GIF (animated), WebP, AVIF, ICO and BMP, plus SVG rendered as `<img>` so its scripts can't run.
    - Shows a checkerboard background, fit/100%/zoom controls, and pixel dimensions, format, byte size and EXIF orientation.
    - Includes Save image… and Copy image.
  - **Video / audio:** `<video>`/`<audio>` preview with metadata.
  - **PDF:** an embedded preview.
  - **Fonts:** a sample-text preview.
  - **Hex:** a virtualised offset/hex/ASCII view for anything binary.
  - **gRPC, protobuf, MessagePack, CBOR (M16):** decoded in Rust (`Engine::decode_body_in`) and shown as the JSON tree with badges (wire types, `bytes`, ext types, tags) by JSON pointer. gRPC lists its messages above the selected one's tree, then the status and trailers. See "M16".
  - **Raw:** the HTTP message with headers, highlighted with a custom CodeMirror stream mode.
- **Delivery to the webview.**
  - A `pmbody://<store>/<flow>/<part>?decoded=1` URI scheme serves the bytes with their real Content-Type. Images, media and PDFs load directly with no base64 and no IPC JSON.
  - The scheme handler sets `Content-Security-Policy` and `X-Content-Type-Options: nosniff`, answers single `Range` requests with 206 (WebKit needs this for media) and keeps the last few bodies in memory for repeated range reads. `max=N` serves only the head.
  - Text viewers still load bytes over IPC (`get_body`, which gained `max`); "Save body to file…" is a `save_body` command using tauri-plugin-dialog from Rust.
- **Large bodies.**
  - Above 2 MB, syntax highlighting falls back to plain text with a "Highlight anyway" button.
  - Above 20 MB, only the first 20 MB is loaded, with a "Load all" button and "Save body to file…".
- **Theme.** The CodeMirror and tree theme share the app's colour tokens. Keys, strings, numbers, booleans/null, tags and attributes each get a token, in both dark and light.

### Colour tokens
Tokens are CSS variables set per `[data-theme]`. Pills use a tinted background with a saturated foreground, at 4.5:1 contrast or better.
- **Methods:**
  - GET blue, POST green, PUT orange, PATCH violet, DELETE red.
  - HEAD/OPTIONS teal, CONNECT grey.
- **Status:**
  - 1xx slate, 2xx green, 3xx cyan, 4xx amber, 5xx red.
  - Active: amber with a pulse. Paused: violet. Failed: red outline. Blocked and Tunnel: grey.
- **URL parts:** scheme dim, host bright, path normal, query dim-accent. The inspector's URL bar colours the host blue (`--url-bar-host`, as Proxyman does) and the path green (`--url-bar-path`); the table keeps them grey.
- **Accent and selection:** macOS system blue (`--primary`, `--ring`, chips, code selection) and the deeper selected-content blue for the selected row, both dark enough for white text at 4.5:1. They were orange until the post-M15 colour pass.
- **Theme:** dark first, with light mode in M10. The body viewers already use tokens for both (`--syntax-*`, `--code-*`, `--checker-*`).
  - (M10) Settings › Appearance picks System, Light or Dark (stored in the webview with the column layout); the native window follows (`setTheme`), so the sidebar vibrancy and traffic lights match. `ui/src/styles/contrast.test.ts` converts every text token and the surface it is drawn on (pill tints composited over the table) from OKLCH and checks 4.5:1 in both themes. Component colours that were hard-coded (overlays, destructive buttons, preview paper, pills on the selected row) became tokens.

## Packaging, signing and updates (M11)
The details, secrets and key rotation are in `docs/RELEASING.md`.
- **Bundle.** The bundle is one universal (`universal-apple-darwin`) `Proxymate.app` and a DMG.
  - Bundle id `dev.proxymate.desktop`, category Developer Tools, minimum macOS 13.3. That is the first release with WebKit 16.4, which the Tailwind v4 CSS (`color-mix()`, `@property`) needs.
  - `bundle.macOS.signingIdentity` is `-` (ad-hoc). `APPLE_SIGNING_IDENTITY` overrides it.
- **Hardened runtime, no App Sandbox, no entitlements.** `Entitlements.plist` is deliberately empty; RELEASING.md lists each exception and why it isn't needed.
  - The bundle holds a single Mach-O, and the same binary runs as `--sysproxy-guardian` and `mcp-stdio`, so one signature covers every executable that gets launched. There are no sidecars.
  - `scripts/release/macos.sh` fails if any Mach-O in the bundle is unsigned or lacks the runtime flag.
- **Signing: ad-hoc, not notarised.** There is no paid Apple Developer account, so releases are ad-hoc signed. Users allow the app past Gatekeeper once ("Open Anyway", or `xattr -dr com.apple.quarantine`); every ad-hoc draft gets those steps from `scripts/release/install-notes-macos.md`, and the README has them. Updates then install from inside the app without the prompt.
  - **Deferred: Developer ID signing and notarisation.** Wired up but dormant until the `APPLE_*` secrets exist (RELEASING.md › "Later"). Then `APPLE_CERTIFICATE` (base64 `.p12`) is imported into a temporary keychain, `APPLE_API_KEY`/`APPLE_API_ISSUER`/`APPLE_API_KEY_PATH` drive notarytool, Tauri notarises and staples the app and signs the DMG, and `macos.sh` notarises and staples the DMG and checks both with `spctl` and `stapler validate`.
- **Updater.** `tauri-plugin-updater`, driven from Rust (`src-tauri/src/updater.rs`).
  - The feed is `releases/latest/download/latest.json` on GitHub, so drafts and prereleases never reach installs. It lists `darwin-aarch64` and `darwin-x86_64`, both pointing at the universal `.app.tar.gz`, which is minisign-signed with the Tauri updater key; the public key is in `tauri.conf.json`.
  - A quiet check runs 5 s after launch, with an opt-out: `UpdateSettings.check_at_launch`, the `updates` row in `app.sqlite`, shown in Settings › Updates.
  - "Check for Updates…" sits in the app menu (Help on Linux/Windows). Native dialogs ask before installing. The restart goes through the normal exit path, so the system proxy is restored first.
  - Update requests bypass the system proxy. Debug bundles with `PROXYMATE_UPDATE_TEST=1` check and install without asking, for end-to-end tests.
- **Release workflow** (`.github/workflows/release.yml`).
  - A `v*` tag builds on `macos-15`, then a separate job merges the `updater-<os>.json` fragments into `latest.json` and uploads everything to a **draft** release. Publishing it is the human step. M12 adds its build jobs beside the macOS one.
  - Manual runs are dry runs, with artifacts only. Pull requests that touch packaging run an ad-hoc build with a throwaway updater key.

## Delivery workflow: one Claude session and one PR per milestone
- **Repository.** M0 creates a new GitHub repository, private by default, called `proxymate`, with `gh repo create`. It's created under the user's account, and the name and visibility are confirmed when M0 starts.
  - `main` is protected: changes land only through PRs, with CI required.
  - M0 commits this plan to `docs/PLAN.md`. Every later session starts fresh and reads it as the source of truth, together with the `CLAUDE.md` that M0 also writes, covering conventions, commands and crate boundaries.
- **One session per milestone.**
  - Each milestone is its own Claude session in its own worktree, on branch `mN-<slug>` (for example `m2-https-h2`) cut from the latest `main`.
  - The session ends by opening a PR titled `MN: <title>`.
  - The PR description lists what shipped, how it was verified (commands and screenshots), any deviations from `docs/PLAN.md`, and follow-ups. If the plan changes, the session updates `docs/PLAN.md` in the same PR.
- **Done means:**
  - CI is green: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, `tsc`, eslint, vitest and the Playwright smoke tests.
  - The milestone's verification steps pass.
  - The PR is open for the user's review. Sessions never merge their own PR.
- **Ordering.**
  - By default milestones run sequentially: each starts after the previous PR is merged.
  - Once M2 is merged, M3 (body viewers), M4 (macOS integration) and M6 (WebSockets) touch mostly separate areas and may run as parallel sessions if desired.
  - M5 (rules) must precede M7 (MCP write tools).
  - After M15 the order is M16, M17, M18, M19, M20, M21 (see "Post-M15 roadmap"). M12 is independent of all of them and can land at any point.
- **Kickoff prompt per session:** "Implement milestone MN from docs/PLAN.md in this repo. Branch from main as `mN-<slug>`, follow CLAUDE.md, meet the milestone's Done criteria, and open a PR."

## Milestones (each is one session and one PR, and ends with something you can demo)
| # | Branch | Scope | Demo / verify |
|---|---|---|---|
| M0 | `m0-skeleton` | New GitHub repo; Cargo and npm workspaces; pm-model; Tauri shell with the AppShell; shadcn/ui init with compact theme and tokens.css; overlay title bar and vibrancy; 100k synthetic rows through the real view → Channel → get_rows path; bindings codegen; CI (macOS and Linux); `docs/PLAN.md` and `CLAUDE.md` | `npm run tauri dev` scrolls 100k rows at 60 fps; CI green |
| M1 | `m1-http-proxy` | HTTP/1.1 forward proxy, live capture, store and segments, ingest and publish, inspector Header/Query/Raw plus a basic JSON pretty view, decoding. CONNECT is tunnelled but not recorded until M2. Synthetic seeding becomes opt-in (`PROXYMATE_SYNTH_FLOWS`) | `curl -x …` flows appear live; testkit integration tests; ingest bench |
| M2 | `m2-https-h2` | CA, leaf cache, CONNECT, ALPN-mirrored MITM, h2, passthrough lists, pinning detection, TLS and Timing tabs | curl `--http2` shows ALPN h2; tunnel and upstream-error tests |
| M3 | `m3-body-viewers` | Everything in "Body viewers and previews": content detection, `pmbody://` scheme, JSON pretty and tree, XML/HTML plus sandboxed preview, form, multipart, GraphQL, JS/CSS, images, media, PDF, hex, large-body handling | Fixture flows for every ContentKind render correctly in dark and light; detection unit tests; Playwright visual snapshots per viewer |
| M4 | `m4-macos-integration` | Cert wizard with handshake verification, system proxy with backup and guardian, app detection and icons, Apps/Domains sidebar | Safari and Chrome decrypt with icons; `kill -9` restores the proxy |
| M5 | `m5-rules` | Rules model, compiler, evaluator, body policy, editor and dry run, presets, breakpoints with editor and queue | Rules integration matrix |
| M6 | `m6-websockets-sse` | WebSocket relay and frame panel, SSE event list | WS echo and SSE tests |
| M7 | `m7-mcp` | All MCP tools and resources, auth, read-only mode, agent badges, audit log, stdio bridge | MCP Inspector; `claude mcp add` end to end |
| M8 | `m8-replay-sessions` | Compose, Replay and Edit & Replay, cURL, HAR 1.2 import/export, `.proxymate` sessions | Golden files; Chrome HAR round trip; save and reopen 100k flows |
| M9 | `m9-device-setup` | Onboarding server, QR codes, LAN IPs, client allow-list, Simulator one-click install, iPhone and Android wizards (emulator is a stretch goal) | Simulator Safari decrypts; physical iPhone decrypts (a manual checklist in the PR) |
| M10 | `m10-polish` | Light-mode pass, keyboard navigation, column persistence, eviction, deep search, DSL autocomplete, error UX | 1-hour soak; stable RSS |
| M11 | `m11-macos-release` | Universal DMG, ad-hoc signing with the hardened runtime, updater, release workflow (Developer ID signing and notarisation deferred: no paid Apple account) | CI builds the universal DMG; it launches after the one-time Gatekeeper override; the updater installs a signed update and refuses a bad signature |
| M12 | `m12-linux-windows` | pm-platform implementations for Linux and Windows, AppImage/deb/rpm, MSI/NSIS, CI matrix | Integration suite green on all three OSes |
| M13 | `m13-visual-rewrite` | Interactive rewrite: editable JSON tree plus header/query/form/GraphQL editors, with diff → actions, an exact JSON splicer, and the Rewrite dialog (see "M13" below) | Edit a captured flow, save it, replay it: only the changed paths are patched |
| M14 | `m14-scripting` | JavaScript scripts on QuickJS: view, rewrite and mock scripts, the Scripts window, script views in the inspector, MCP script tools, session format 2 | A base64 JSON field shows decoded in the inspector on live and saved flows; script errors show while flows pass; timeouts and memory limits hold |
| M15 | `m15-context-menus` | Right-click menus with copy actions on every surface that shows data (see "Context menus (M15)"), one clipboard helper with toasts, the native-menu policy | Header, URL, body, hex, flow-cell, WebSocket, MCP-token and rule-row menus copy the right text (`e2e/menus.spec.ts`); copy formats unit-tested; a release build shows no native menu on blank space |
| M16 | `m16-protobuf-grpc` | gRPC and gRPC-web framing and trailers, schema-less protobuf decoding, `.proto`/descriptor schemas, MessagePack and CBOR viewers (see "M16") | `grpcurl` through the proxy shows decoded unary and streaming calls with and without a schema, and the gRPC status; zoo snapshots for protobuf, MessagePack and CBOR |
| M17 | `m17-workflow` | Multi-select and bulk actions, delete flows, labels and comments (kept in sessions, with pins), header columns, Compare two flows, copy as code, Repeat with iterations, search highlighting in bodies (see "M17") | e2e for multi-select, labels, header columns and Compare; golden files per code language; a 1,000-iteration repeat reports the right counts |
| M18 | `m18-network` | Upstream proxies (HTTP and SOCKS5, per host, with auth), a SOCKS5 listener, reverse proxies, client certificates, DNS overrides, Settings › Network (see "M18") | curl through Proxymate chained to a testkit proxy, `curl --socks5-hostname`, a reverse proxy on a local port, an mTLS upstream and a DNS override, all as integration tests |
| M19 | `m19-websocket-rules` | Rules on WebSocket handshakes, message actions (rewrite, drop, delay), message breakpoints, sending and resending messages, `onWsMessage` scripts (see "M19") | Against the testkit echo: a rule rewrites a JSON field in text messages, a breakpoint edits one, the UI and MCP inject one, order is kept |
| M20 | `m20-offline-mocking` | Serve from a recorded session or HAR, Charles session import, OpenAPI import (operation labels and mock rules) (see "M20") | Record against the testkit upstream, stop it, "Mock from session" answers the same; the Charles fixtures import; the Petstore spec's rules answer |
| M21 | `m21-capture-targets` | Intercepted terminal and browser launchers, recording allow/block lists, No Caching and Block Cookies toggles, auto-save (see "M21") | A launched Chrome decrypts without keychain trust; node, python and curl in the launched terminal decrypt; a never-recorded host makes no flow; an auto-saved session opens |

## Post-v1 milestones: M13 visual rewrite and M14 scripting

### Context
The user asked for two features beyond v1:
1. **Scripting** (M14). The motivating case is a request that sends a JSON object base64-encoded; the user wants it decoded automatically in the inspector.
2. **Visual rewrite** (M13). "Rewrite…" on a flow shows the captured request and response as editable tables and trees, instead of making the user type JSON pointers.

**Decisions the user made:**
- Scripts are JavaScript on embedded QuickJS, with view/decoder, traffic-modifying and mock scripts, and no network or file access.
- Saving a rewrite patches **only what changed**; a "Replace whole body" toggle replaces the body instead.
- Editable types: JSON, headers, query parameters, form-urlencoded bodies and GraphQL variables. XML is not included.

**Order and parallel work.** M13 merges first and M14 rebases onto it. M13's `Action` variants sit right after `JsonRemove`; M14 appends `Script` as the last variant. M13 owns `eval.rs`'s `edit_body`, `rules/json_edit.rs` and the `*_in` dry runs; M14 only makes the phases async and adds a Script arm. `bindings.ts` is regenerated, never hand-merged. M12 is independent of both.

**Facts that shaped the designs:**
- **No serde_json feature changes.** `arbitrary_precision` breaks numeric fields inside internally tagged enums (`Action`, `RuleOrigin`, `BreakpointResolution`) and leaks into Tauri and rmcp; `preserve_order` reorders keys on remove. Exact JSON edits go through a byte splicer and `JsonSet.raw` instead.
- Re-serialising a body (the pre-M13 `JsonSet`) sorts keys and rewrites numbers; a JS object reorders integer-like keys (`parseLossless` alone isn't enough for editing).
- Saved sessions are versioned (`store/sqlite.rs`, `VERSION = 1`, strict); M13 changes no postcard layout. (Now version 6: see M14, M16, M17, M18 and M19.)

### M13: Visual rewrite (`m13-visual-rewrite`)

**Model (`pm-model/src/rules.rs`)**
- `JsonSet` gains `#[serde(default)] raw: Option<String>`: the value as exact JSON text, which takes precedence over `value` (validated on compile).
- After `JsonRemove`: `FormSet{phase,name,value}`, `FormRemove{phase,name}` (urlencoded bodies, like `SetQuery` on the body) and `AddHeader{phase,name,value}` (appends, so repeated headers are a `RemoveHeader` then several `AddHeader`s).
- `RuleTestResult` gains `request_body_truncated` / `response_body_truncated`.
- GraphQL variables need no new action: they are `JsonSet`/`JsonRemove` under `/variables` (`/0/variables` for a batch), or a `SetQuery` of `variables` for a GET.

**Engine**
- **JSON splicer** (`rules/json_edit.rs`), used by `eval.rs` for every `JsonSet`/`JsonRemove`:
  - The body is validated once (serde_json into `IgnoredAny`), then scanned to the value the pointer addresses and only those bytes are replaced. Key order, number text and formatting elsewhere stay byte-identical.
  - An inserted member copies its last sibling's whitespace and `:` style (in an empty container, the document's indent unit and colon). A removed member takes its comma with it. Missing parents are created as nested objects.
  - Same semantics as the old parse-and-reserialise code, including repeated keys (the last one is edited; a removal removes them all) and `-`/length appends. A differential test against that code (kept as the test reference) runs thousands of generated documents and edit sequences in compact, pretty and tab styles.
  - A run of consecutive JSON actions in a rule is applied in one back-to-front pass when the edits can't interact (no overlapping or touching splices, nothing else in an array whose length one edit changes, no failures or no-ops); otherwise one by one.
- **Store-aware dry runs:** `draft_rule_in(store, …)`, `test_rule_in(store, rule, flow, body_limit)` (the result's bodies are cut at the limit, 256 KB by default, 2 MB from the dialog; the rule runs on whole bodies) and `flows_matching_rule(store, rule, limit)` (newest first; header and response conditions checked on the detail). The live-only versions wrap them; the Tauri commands take `store`.
- The Rewrite preset drafts an empty action list.

**JSON document and editor**
- `lib/json/doc.ts`: ordered nodes `obj{entries[{id,key,v}]}`, `arr{items[{id,v}]}`, `str`, `num{raw}`, `bool`, `null`, built by a builder mode of the `parseLossless` tokenizer (in the JSON worker above 256 KB). Ids are stable through renames and array edits; edits are immutable (path copying). `align` gives a dry run's result the original's ids (members by key, items by index).
- `components/JsonEditor/`: virtualised rows, inline typed editors (numbers checked against the JSON grammar and kept as typed), change type, add, remove, rename, duplicate, reset to original, undo/redo (200 steps, `hooks/useUndoable.ts`), rows tinted against the original with the `--diff-*` tokens (contrast-tested), "replaced as a whole" and "−N removed" badges. Keys: ↑↓ ←→, ↩ edit, F2 rename, ⌘↩ add, ⌘D duplicate, ⌫ remove, ⌘Z/⇧⌘Z, Esc cancels. Bodies over 5 MB are edited as text.

**Edits → actions (`lib/rewrite/diff.ts`, `lib/rewrite/session.ts`, table-tested)**
- A changed scalar is `JsonSet{raw}` (numbers compared by text); a removed member `JsonRemove`; an added one a `JsonSet` of its subtree; a rename both (removals come first, so swaps work).
- Arrays: appended items are `JsonSet …/-`; any other insert, removal or reorder sets the whole array (badged in the tree).
- A new root type, or "Replace whole body", is `SetBody`. Above 100 actions the dialog suggests the toggle.
- Headers (case-insensitive names): `SetHeader` / `RemoveHeader`; a repeated header is `RemoveHeader` plus an `AddHeader` per value. `Content-Length`, `Content-Encoding` and `Transfer-Encoding` are read-only.
- Query and form fields: `SetQuery`/`RemoveQuery`, `FormSet`/`FormRemove`; a repeated name can be removed, not edited. Status: `SetStatus`. Other text bodies: "Replace whole body" (`SetBody`) or find & replace pairs (`ReplaceBody`, exact).

**Rewrite dialog (`components/Rewrite/`)**
- Opens from "Rewrite…" in the flow menu (live flows and documents), a Rewrite button in the inspector URL bar, "Edit visually" in the Rules Editor, and the palette.
- Request side: Headers, Query, Body. Response side: Status, Headers, Body. The Body tab is the JSON tree, the GraphQL variables tree, a form table or text.
- Matcher chips: Exact URL, Ignore query (default), Path glob (id-like segments become `*`), Any method, Advanced… (the `MatcherBuilder`).
- Live preview: a debounced `test_rule_in` on this flow, showing the status, a header key/value diff and a line diff of the body (pretty-printed in order with exact numbers), scrolled to the first change, with a note when the body was cut. An Actions tab lists what will be saved.
- Saving makes a rule with origin `Preset{Rewrite}` (or updates the rule being edited), editable in the Rules Editor.
- Reopening a saved rule: on the selected flow if it matches, else the newest from `flows_matching_rule`. Its visual actions are dry-run and the edits rebuilt from the result (`align`). Their paths are pinned, so a `JsonSet` equal to the flow's value is kept, and sets that created parents stay at their paths. Other actions (Delay, Breakpoint, ReplaceHeaderValue…), and visual ones the rebuilt edits don't account for, are kept unchanged and listed read-only.
- The Breakpoint Editor has a Text/Tree toggle for JSON bodies. `GraphQlView` shows variables through the document model (member order and numbers as sent). The Rules Editor's `JsonSet` form edits the exact `raw` text.

**MCP**
- `create_rewrite` gains `json_remove[]`, `json_set[].raw`, `set_query`/`remove_query`, `form_set`/`form_remove` and `add_request_headers`/`add_response_headers`.
- New read-only tool `test_rule{rule_id | rule, flow_id, store?}`.

**Tests:** the splicer's unit and differential tests; `FormSet`/`AddHeader`/raw compile and evaluate; an integration test where a dialog-shaped rule runs against an upstream whose other fields (and formatting, and 64-bit ids) differ, which come through byte-identical; `test_rule_in`/`flows_matching_rule` on a saved session; an rmcp test of `test_rule` and the sugar; vitest for the document, diff, matcher chips, line diff and rebuilding with pins; Playwright for the dialog on the zoo JSON flow and the Breakpoint Editor tree, in dark and light.

### M14: Scripting (`m14-scripting`)
The design as built is "Scripting (M14)" above (with the rules evaluation order, session format 2 and the MCP script tools in their sections).

## Post-M15 roadmap: M16–M21

### Context
After v0.3.0 the user asked for a feature audit against Proxyman, Charles, HTTP Toolkit, mitmproxy and Fiddler (a Proxymate mobile app is out of scope). Proxymate already leads on table and filter performance, the depth of its MCP server, lossless JSON rewrites and deep search. The gaps fell into six groups, which the user put in this order: protobuf and gRPC (M16), everyday workflow (M17), network plumbing (M18), WebSocket rules (M19), offline mocking (M20), capture targets and noise (M21). M12 (Linux and Windows) stays as planned and is independent of all six.

**Rules that apply to every milestone here:**
- New `Action` variants go after `Script`. That changes the postcard `FlowDetail`, so bump the session `VERSION` in `store/sqlite.rs` (each milestone takes the next one; every older version still opens) and move CLAUDE.md's "last variant" note.
- New `ContentKind` variants go after `Binary`, for the same reason (`BodyInfo.kind` is in the detail).
- New `FlowRow` fields are appended, with `decodeRow` and its test updated.
- New settings are `app.sqlite` rows owned by `pm-core/src/settings.rs`, with a Settings section and Engine methods; adapters still call only `Engine`.

### M16: Protobuf and gRPC (`m16-protobuf-grpc`)
- **Detection (`io/content.rs`).** New kinds after `Binary`: `Grpc` (`application/grpc`, `+proto`, and `application/grpc-web`, `+proto`, `-text`), `Protobuf` (`application/x-protobuf`, `application/protobuf`, `application/vnd.google.protobuf`), `MessagePack` (`application/msgpack`, `x-msgpack`) and `Cbor` (`application/cbor`). They get filter chips under Other and DSL `ct:` names.
- **gRPC framing.** A body is split into length-prefixed messages (1-byte compressed flag, 4-byte length). Each message is decompressed according to `grpc-encoding` (identity, gzip, deflate, zstd). For gRPC-web, `-text` bodies are base64-decoded first, and the trailer frame (flag `0x80`) is parsed in place of HTTP trailers. Streaming calls are split as their bytes arrive, so server streams stay live like SSE.
- **Trailers.** HTTP/2 response trailers are recorded in `FlowDetail`, at the end, so the session `VERSION` bumps. `grpc-status`, a percent-decoded `grpc-message` and `grpc-status-details-bin` (decoded as `google.rpc.Status`) are shown. `FlowSummary.grpc_status` (an appended `FlowRow` field) lets the Status pill read `OK` or `NOT_FOUND`, and adds a DSL `grpc:` term (`grpc:!0` for failures).
- **Schema-less decoding.** The protobuf wire format is decoded to a tree of field numbers, wire types and values: varint as signed or unsigned, fixed32/64 as numbers and floats, and length-delimited fields tried as a nested message, then as UTF-8, then as bytes. Packed repeated fields are guessed.
- **Schemas.** Users import `.proto` files (compiled in Rust with `protox`, no `protoc` needed) or a `FileDescriptorSet` (`buf build -o`, `protoc --descriptor_set_out`). They live in an `app.sqlite` `proto_schemas` table. Decoding uses `prost-reflect` into proto3 JSON.
  - A gRPC path (`/pkg.Service/Method`) finds its request and response types by itself.
  - Plain protobuf over HTTP, and binary WebSocket messages, need a mapping (a `Matcher`, a part and a message type), set from the viewer's "Decode as…".
  - Server reflection (`grpc.reflection.v1`) is a follow-up.
- **Viewers.** The gRPC viewer lists the messages (index, direction, size, compressed) with each one in the read-only JSON tree, plus the trailers. A protobuf body uses the same tree, and schema-less fields show their numbers and wire types. MessagePack (`rmp-serde`) and CBOR (`ciborium`) become JSON trees, with bytes, ext types and tags badged. All decoding runs in Rust (`Engine::decode_body`), cached like view scripts, so MCP gets the same output.
- **MCP.** `get_flow_body` gains `format: "decoded"` for these kinds, and there is a read-only `list_proto_schemas`. Rewriting protobuf (re-encoding edited JSON) is a follow-up; breakpoints show the decoded form read-only and edit the bytes as hex.
- **Tests.** pm-testkit gains a tonic gRPC upstream (unary, server streaming, an error status, gzip messages, and a gRPC-web endpoint), and the zoo gains protobuf, MessagePack and CBOR fixtures. There are unit tests for framing, the schema-less decoder and `.proto` import; Playwright snapshots each viewer in dark and light.

**As built (M16).** Where it differs from the above, or adds detail:
- **Model.** `ContentKind` gains `Grpc`, `Protobuf`, `MessagePack`, `Cbor` after `Binary`; `FlowDetail.trailers: Option<Vec<Header>>` is its last field; `FlowSummary.grpc_status: Option<u8>` is `FlowRow`'s last field. Session format **3** appends `trailers` and a `grpc_status` column; versions 1 and 2 still open (a `None` is one postcard byte, appended on read; the column reads as `NULL`). New IPC types are in `pm-model/src/protobuf.rs` (`DecodedBody`, `DecodedMessage`, `Badge`, `GrpcStatus`, `ProtoSchema`, `ProtoMapping`).
- **Trailers and status.** `TeeBody` hands trailer frames to `FlowRecorder::trailers` (response side only). A trailers-only response (tonic sends errors this way) keeps its status in the response headers and has no trailers. For gRPC-web, the trailer frame of a response body up to 8 MB is read when the flow finishes and kept as the flow's trailers, so the Headers tab, the DSL and sessions see it like HTTP/2 ones. A `FlowEvent::Grpc` puts the status in the summary. The Status pill shows the name (`OK`, `NOT_FOUND`) coloured like 2xx for OK, 5xx for server faults (UNKNOWN, DEADLINE_EXCEEDED, INTERNAL, UNAVAILABLE, DATA_LOSS) and 4xx otherwise, with the HTTP status in its tooltip.
- **Framing (`io/grpc.rs`).** Splitting works on any prefix of a body: whole frames decode, the rest is `pending_bytes`. `-text` bodies are Base64 per padded chunk. Per-message decompression reuses `io/decode.rs` (gzip, deflate, zstd; capped). `application/grpc+json` messages are shown as JSON. `grpc-status-details-bin` is decoded by hand as `google.rpc.Status`, each `Any` keeping its type URL with its value schema-less.
- **Schema-less (`io/wire.rs`).** Printable UTF-8 wins over a nested-message parse (short text often parses as a message; a real message is rarely all printable), then a nested message, then Base64 bytes. Packed varints are suggested in the badge (`bytes · 3 · packed? 0, 1, 2`) rather than replacing the value; a zigzag reading is offered only for odd values up to 32 bits; fixed64/fixed32 show as doubles/floats when the value is a plausible magnitude. Groups are decoded; nesting is capped at 48. JSON text is written by `io/tree.rs`, in field order, since serde_json's map sorts keys and `preserve_order` stays off.
- **MessagePack and CBOR (`io/objects.rs`).** MessagePack uses `rmpv` (rmp's value type, which keeps ext types) rather than `rmp-serde`; CBOR uses `ciborium::Value`. Back-to-back values (a MessagePack stream, a CBOR sequence) become one message each. Ext type −1 is shown as a timestamp; well-known CBOR tags are named; non-string map keys are stringified and badged.
- **Schemas (`protos.rs`).** `proto_schemas(id, name, files, descriptor, added_at_ms)` stores each import compiled (imports included) as a `FileDescriptorSet`; all of them make one `prost-reflect` pool (shared files once; a conflicting one is refused at import). `.proto` files from disk are named relative to their common folder, with each file's folder as an include path. The mappings are a settings row (`proto_mappings`), validated against the pool and compiled through the rules compiler. "Decode as…" saves a mapping for the flow's exact host and path and that part. A protobuf `Content-Type` parameter `messageType=`/`proto=`/`type=` naming a known type also applies. A mapping beats a gRPC path. Typed messages are proto3 JSON with default fields included (64-bit integers as strings, so exact); unknown fields are counted in a badge; a payload that doesn't fit its type falls back to schema-less with the error shown.
- **Decoding cache.** `Engine::decode_body_in(store, flow, part)` works on live flows and documents. Results are cached (64 MB) by store, flow, part, bytes seen, trailers and schema generation, so a live stream re-decodes only when it has grown. The UI polls it every 500 ms while the flow is active (decoded viewers render before the body finishes, unlike the others).
- **MCP.** `get_flow_body` with `format: "decoded"` pages by message (`offset` is the first message index; messages are added up to `max_bytes` of JSON) and applies `json_pointer` inside each message; badges come back as a `{pointer: label}` map. `get_flow` includes the response's `trailers`; flow summaries include `grpc_status`. `list_proto_schemas` returns schemas and mappings.
- **UI.** Viewers `grpc` (gRPC) and `decoded` (protobuf, MessagePack, CBOR) with source `"decoded"`; the JSON tree takes badges. Trailers also show under the response headers. Settings › Protobuf imports (`.proto`, `.pb`, `.binpb`, `.desc`, `.protoset`) and removes schemas and mappings. The DSL gains `grpc:` and `ct:grpc|protobuf|msgpack|cbor` (autocompleted).
- **Testkit.** `pm-testkit/proto/shop.proto` (`shop.v1.Orders`: `GetOrder`, with `"missing"` → NOT_FOUND plus details; `ListOrders` server stream) is compiled by protox in `build.rs` and served by tonic over TLS from a test CA, h2 and gRPC-web (h1/h2) on one port, gzip and zstd when the client accepts them. `cargo run -p pm-testkit --bin upstream -- --grpc [PORT]` runs it by hand. The zoo's binary fixtures are generated by `zoo::binary_fixtures` and the engine's decoded output for each (what the UI mock serves) by `pm-core/tests/zoo.rs`; `UPDATE_ZOO=1` rewrites both.
- **grpcurl.** Go never proxies `localhost` or loopback addresses, so the manual check uses `localhost.` (`HTTPS_PROXY=http://127.0.0.1:9090 grpcurl -cacert <Proxymate CA> -import-path crates/pm-testkit/proto -proto shop.proto localhost.:<port> shop.v1.Orders/GetOrder`).
- **Not done (follow-ups).** The Breakpoint Editor still shows these bodies as text: the read-only decoded view and hex editing are left for later. Plaintext gRPC (h2c prior knowledge) through a CONNECT isn't intercepted. Server reflection and protobuf rewrites stay follow-ups, as planned.

### M17: Workflow (`m17-workflow`)
- **Multi-select.** The selection is flow ids, not positions: ⇧-click selects a range, ⌘-click toggles, and ⌘A selects the whole view. A whole-view selection is stored as `{view, except}`, so 100k rows never materialise. The inspector follows the focused row. Bulk actions take a `FlowSelection` (`ids` or `view` plus `except`): copy URLs or cURL, Export HAR, Save Session, pin, label, replay in order, and delete.
- **Delete flows.** `Engine::remove_flows_in(store, selection)` reuses eviction's gaps and segment bookkeeping (`Store::evict`, `SegmentStore::drop_through`), so disk is freed when no kept flow still uses a segment. ⌫ deletes the selection; ⌘⌫ still clears.
- **Labels and comments.** `FlowSummary.label` (six colours, `--label-*` tokens added to the contrast test) and a comment per flow. Sessions save them, and pins too, in a new `annotations` table, which bumps the session `VERSION`; this removes the "a saved session doesn't keep pins" limitation. New DSL terms: `label:red`, `is:commented`, `comment:"…"`. The Edited column becomes a Notes column that shows the label dot and a comment icon too.
- **Header columns.** "Add column" in the header's context menu takes a request or response header name (up to 8, `ColumnSettings` in prefs). Ingest copies those values into a per-store column table; existing flows are backfilled in the background, like deep search. `FlowRow` gains a trailing `extra: Vec<Option<String>>`. Header columns can be sorted and get "Filter by".
- **Compare.** "Compare" on two selected flows, from any stores, opens a window with the request line, status, headers (a key/value diff, with an option to ignore volatile headers such as `date`), bodies (the M13 line diff over pretty-printed, order-kept text) and timing.
- **Copy as code.** `io/snippets.rs` renders a `RequestSpec` as JavaScript `fetch`, Python `requests`, HTTPie, Go `net/http`, Swift `URLSession`, Kotlin OkHttp and PowerShell, with golden files beside cURL's. Snippets appear under the Copy submenu and as an MCP `export_code{id, language}`.
- **Repeat.** "Repeat…" (⌥⌘R) sets iterations (up to 10,000), concurrency (up to 64) and a delay. `Engine::repeat(spec, RepeatOpts)` sends through `Engine::send` and pushes progress. Its flows get `sent_by: Replay` and a repeat group, and a summary shows status counts and min, p50, p95 and max. MCP gets a write tool, `repeat_flow`.
- **Search highlighting.** When the filter has `body:` or bare-text terms, matches are highlighted in CodeMirror viewers and the JSON tree, with next and previous buttons.

**As built (M17).** Where it differs from the above, or adds detail:
- **Model.** `FlowSummary` gains `label: Option<FlowLabel>` (red, orange, yellow, green, blue, purple), `comment: Option<Arc<str>>` and `extra: Vec<Option<Arc<str>>>` (header column values); `FlowRow` appends `label, comment, extra`, with `extra` last. `FlowSelection::View` gains `except` (`#[serde(default)]`); `SortKey::Extra` with `SortSpec.extra` (the column index) sorts by a header column. `SentBy` gains `Repeat`. New IPC types are in `pm-model/src/workflow.rs` (`HeaderColumn`, `RepeatOpts`, `RepeatProgress`, `StatusCount`, `CodeLanguage`). `FlowSummary::url()` builds a flow's URL.
- **Delete.** `Engine::remove_flows_in` skips flows still in flight (their recorders would write them back) but deletes pinned ones. Segments now remember their oldest writer as well as their newest, so a delete frees every finished segment up to the newest deleted id that no remaining flow could have written to (`SegmentStore::in_use_through`, then `drop_through`); eviction's `kept` list would have meant reading every older flow's detail. A document's deletes change only its tab, not its file. ⌫ deletes the selection (only while the flow table has the focus); ⌘⌫ still clears.
- **Annotations.** Labels, comments and pins go through ingest for the live store (`FlowEvent::Label`, `Comment`, `Pin`) and straight into a document's store otherwise, so they work on saved sessions and HARs too (`pin_flows_in`, `label_flows_in`, `comment_flow_in`). Session format **4** adds `annotations(flow, pinned, label, comment)`; versions 1–3 still open (without annotations). HAR exports carry the comment in the entry's `comment` (over the "Sent from Proxymate" note); HAR imports read it back. DSL: `label:red,blue` (or `any`), `is:labelled`, `is:commented`, `comment:"…"`. The Notes column (replacing Edited) shows the label dot, a comment icon and the edited pencil; the inspector shows the label and comment under the URL bar. MCP flow summaries include `pinned`, `label` and `comment`.
- **Header columns.** The UI keeps them in prefs (`headerColumns`, up to 8) and hands them to `Engine::set_header_columns`, which is global (every store). Each change bumps a generation: every store's values are dropped (`FlowEvent::Columns`), recorders send new flows' values as their request and response heads arrive (`FlowEvent::Extra`, ignored if stale), and a `pm-columns` thread backfills existing flows from their details, 512 at a time, writing only headers it found so it never clears a value a recorder set meanwhile. Documents opened later are backfilled on open. Values are cut at 256 characters. Column ids are `hdr:<part>:<name>`; "Filter by" adds a `reqheader:`/`resheader:` term.
- **Compare.** A large dialog rather than a separate window (it works in the browser UI and Playwright alike). The bulk menu's Compare takes two selected flows; "Mark for Compare" then "Compare with Marked" compares flows from different tabs. Headers are compared by name (case-insensitive, repeated ones joined), with "Ignore volatile headers" (date, request ids, traces) on by default; bodies use the M13 line diff over `prettyForDiff`; timing shows each phase and the difference.
- **Copy as code.** `io/snippets.rs` renders the flow's editable `RequestSpec` (body decoded, repeated headers joined). Golden files are `tests/golden/snippets/<language>/<case>.<ext>` (four cases per language, `UPDATE_GOLDEN=1`); cURL keeps its own module and golden files. `Engine::code_snippet(store, id, language)`; MCP `export_code{id, language}` (Read).
- **Repeat.** `Engine::repeat(spec, RepeatOpts, sink)` (`repeat_as` for an agent's origin) runs N workers on the tokio runtime, each sending through `Engine::send` with `wait`, and pushes `RepeatProgress` at most every 100 ms and once at the end: counts by status (`None` for failures), min/p50/p95/max, and the first and last flow id. Its flows are `SentBy::Repeat` (a "Repeat" badge) rather than a stored repeat group. A spec that can't be sent stops it with the error; `cancel_repeat` lets in-flight requests finish. MCP `repeat_flow{id, iterations, concurrency, delay_ms, apply_rules}` (Write) waits for the end and returns the counts; cancelling the call cancels the repeat.
- **Selection.** `lib/selection.ts`: ids, or `all` with exceptions (⌘A while the table has the focus). ⇧-click and ⇧-arrows select a range from the anchor (ids fetched with `get_rows` when the rows aren't loaded; a range covering the whole view becomes `all`); Escape drops back to the focused flow; a right-click inside a multi-selection opens the bulk menu. The footer shows how many are selected.
- **Search highlighting.** `bodySearchTerms` (`lib/dsl.ts`) takes `body:`, the side's `reqbody:`/`resbody:` and bare words; the Body tab provides them to its viewers. CodeMirror viewers mark matches with a `MatchDecorator` (visible lines only) and the JSON tree marks keys and values; both have a match bar (count, previous, next) that selects and scrolls to each match, expanding the tree as needed.
- **Commands.** New ⌘K commands for every bulk action, the labels, Comment…, Compare, Repeat… (⌥⌘R), Copy as each language, Add header column…, and Show labelled / commented flows. `AppCommand` gains `alt`, `bare` (no ⌘) and `inTable`.

### M18: Network plumbing (`m18-network`)
- **One dialer.** `proxy/dial.rs` owns every upstream connection: the MITM's `dial_tcp`, the plain-HTTP pool (a custom connector replaces `HttpConnector`), `send.rs`, WebSockets and Map Remote. The trust probe and the onboarding server never dial. The dialer applies DNS overrides, then upstream-proxy routing, then client certificates (when TLS is set up). `FlowDetail` records the route it took (`via`, at the end; the session `VERSION` bumps), and the TLS and Timing tabs show it.
- **Upstream proxies.** `UpstreamSettings` holds rules in order, each a list of host globs and a proxy: `Http{host, port, auth}`, `Socks5{host, port, auth}` or `Direct`, plus a bypass list.
  - HTTP upstreams get `CONNECT` for TLS and WebSockets and absolute form for plain HTTP. SOCKS5 resolves names remotely.
  - Auth is Basic or SOCKS username/password. The secret is kept by a new `pm-platform` `SecretStore` trait (the keychain on macOS, a 0600 file elsewhere).
  - "Use the proxy that was set before Proxymate" reads the system proxy backup, which matters when Proxymate is the system proxy on a corporate network.
  - A loop guard refuses an upstream that is one of our own addresses and ports.
  - PAC files and NTLM/Kerberos are follow-ups.
- **SOCKS5 listener.** On the same port: `sniff.rs` routes a leading `0x05` byte to a SOCKS5 handshake (no auth, CONNECT only, IPv4, IPv6 and domain targets; `Engine::admit` and the LAN allow-list apply as before). The stream then goes through the CONNECT path, so the SSL policy decides between MITM and passthrough. Plain HTTP is sniffed, and anything else becomes a recorded TCP tunnel flow.
- **Reverse proxies.** Each `ReverseProxy{listen_port, target_url, preserve_host, tls}` is its own listener. With `tls`, the listener serves a leaf for `localhost` and its addresses. Its flows record the target URL, the Client column shows "Reverse :<port>", and rules apply as usual.
- **Client certificates.** `ClientCertSettings` maps host globs to a PKCS#12 file (its password in the `SecretStore`) or PEM files. The upstream `ClientConfig` uses `with_client_auth_cert`, cached per host and identity. A handshake that ends in `certificate_required` explains itself in the failure banner with a link to the setting.
- **DNS overrides.** Host globs map to IPs, applied before resolution. SNI and `Host` don't change. A DoH resolver is a follow-up.
- **UI and MCP.** Settings › Network has Upstream, SOCKS, Reverse Proxies, Client Certificates and DNS. The toolbar's "Listening on …" menu lists the extra listeners. `get_status` reports them read-only; there are no MCP write tools for network settings.
- **Tests.** pm-testkit gains an HTTP CONNECT proxy with Basic auth, a SOCKS5 server and client, and an mTLS upstream that requires a client certificate from its test CA. Integration tests cover chaining, auth failures, the loop guard, SOCKS into MITM, a reverse proxy over plain and TLS, mTLS, and DNS overrides for a name that doesn't exist.

**As built (M18).** Where it differs from the above, or adds detail:
- **Model.** `pm-model/src/network.rs`: `NetworkSettings { upstream: UpstreamSettings { rules, bypass, use_system_previous }, reverse_proxies, client_certs, dns }`, saved as one `network` settings row (owned by `settings.rs`; compiled in `pm-core/src/network.rs`). `UpstreamProxy` is `Direct | Http(ProxyServer) | Socks5(ProxyServer)`; `ClientIdentity` is `Pkcs12 { path, password } | Pem { cert_path, key_path }` (empty `key_path`: the key is in the certificate file). Passwords are write-only: a save moves them to the `SecretStore` and the row keeps `has_password` (`None` keeps the saved one, `""` deletes it); secrets a save no longer refers to are deleted. The default bypass list is `localhost`, `127.0.0.1`, `::1`, `*.local`. `FlowDetail.via: Option<Via { proxy, addr, dns_override, client_cert, reverse_port }>` is its last field: session format **5** (versions 1–4 still open; `None` is one postcard byte, appended on read). `ProxyStatus.listeners: Vec<ListenerStatus>` lists SOCKS5 and each reverse proxy, which `get_status` reports as part of `proxy`.
- **Secrets.** `pm-platform/src/secrets.rs`: `SecretStore` with `KeychainSecrets` (generic passwords, service `Proxymate`, one account per key) on macOS, `FileSecrets` (a 0600 JSON file) and `MemorySecrets`. Only the app uses the keychain (`EngineConfig::use_keychain`); other engines (the CLI, every test) use `<data dir>/secrets.json`, or memory without a data dir, so tests never touch the keychain. Passwords and PKCS#12 identities are read on first use, so launching never waits on the keychain; a save loads every identity at once to report a wrong path or password.
- **Dialer.** `proxy/dial.rs::dial(engine, host, port, Purpose)`: DNS override (by host glob, also applied to proxy host names and to what a proxy is asked for), then the hop (first enabled rule, bypass first; with `use_system_previous`, unmatched hosts use the backup's secure entry for tunnels and its web entry for plain HTTP, honouring its bypass list and never for loopback), then the loop guard on the address actually connected (`Engine::is_own_endpoint`: our main or reverse ports on any local address). `Purpose::Tunnel` gets `CONNECT` from HTTP upstreams (the response head is read a byte at a time); `Purpose::Http` (the pool) connects to the proxy and sends absolute form, with `Proxy-Authorization` added by `Exchange::send`. SOCKS5 upstreams get names, not addresses (remote resolution). A proxy's handshake counts as connect time. Errors carry a SOCKS reply code for the SOCKS listener. The connect timeout is Settings › Advanced's (it was a fixed 15 s for the MITM).
- **Pool.** The plain-HTTP client is one per engine (`Engine::pool`), built on first use with `dial::Connector` and dropped when the proxy or network settings change, so idle connections never outlive a route change. Each pooled connection hands its responses a `PoolInfo` (via `Connected::extra`): its `Via`, and its setup timing for the first flow only (plain HTTP flows now show DNS and connect times too).
- **WebSockets** in absolute form (`ws://`) no longer use the pool: each handshake dials its own connection (`mitm::open_h1`), which it then takes over, so HTTP upstreams see `CONNECT` as designed. `open_h1` + `send_once` also serve rules that send a request to another origin (`send_direct`).
- **Client certificates.** `TlsContext::client_config(alpn, insecure, identity)` sets `client_auth_cert_resolver` to the identity (`rustls::sign::CertifiedKey::from_der`, which checks the key fits); the `ClientConfig` is per connection anyway. PKCS#12 is read with `p12-keystore` (pure Rust, legacy PBES1 included). With TLS 1.3 a server's `certificate_required` alert arrives after the handshake, often as the connection task's error rather than the request's ("channel closed"); each upstream connection keeps its task's error (`mitm::Died`) so the flow says "<host> asks for a client certificate. Add one … in Settings › Network › Client Certificates." The failure banner offers "Network settings…".
- **SOCKS5 listener.** `proxy/socks.rs`, after `Engine::admit` (a refused LAN client gets "no acceptable method"). `mitm::decide` is the decision CONNECT and SOCKS share (probe, onboarding, passthrough, intercept). Passthrough dials before replying, so failures get their SOCKS reply code. Intercepted connections sniff their first bytes: TLS goes to the MITM, HTTP is served as HTTP, anything else is a recorded tunnel (`raw_tunnel`). A SOCKS client silent for 2 s is relayed raw (server-first protocols); `CONNECT` keeps its 30 s wait. The CONNECT path also tunnels non-TLS, non-HTTP bytes now instead of failing to parse them.
- **Reverse proxies.** `proxy/reverse.rs`: one loopback listener per enabled entry, started with the first main listener and restarted on every save; each flow goes through `forward` (which now takes `https://` targets and records `Via.reverse_port`), so rules and WebSockets work. The client is `PeerClient::known("Reverse :<port>")`. With `tls` the listener mirrors ALPN (h2 or HTTP/1.1) and serves a leaf for the SNI, or for the local address without one. HTTPS targets use a one-off connection per request (`send_direct`); pooling them is a follow-up. Bind failures show per listener in Settings › Network and `get_status`. Checks refuse ports below 1024, duplicates, the main port and targets that point at a Proxymate listener.
- **UI.** Settings › Network (between Proxy and SSL Proxying) edits a draft and saves the whole page at once, with errors in its sticky Save bar; passwords show only "Saved in the keychain". The toolbar's listening menu copies the SOCKS5 address and each reverse proxy, and opens Network settings. The Timing tab lists Route, Connected to, DNS override and the reverse port; the TLS tab adds Upstream proxy and Client certificate. The mock backend has the page's data and checks (`lib/mockNetwork.ts`); `e2e/network.spec.ts` covers save, refusal and reload.
- **Not done (follow-ups).** PAC files, NTLM/Kerberos, a DoH resolver, pooling HTTPS reverse-proxy targets, a file picker for certificate paths, and DNS overrides for plain HTTP through an HTTP upstream (the proxy resolves the absolute-form URL itself).

### M19: WebSocket rules (`m19-websocket-rules`)
- **Handshakes.** The upgrade request goes through `proxy/exchange.rs`'s request phase: header and query actions, Map Remote, Block and request breakpoints apply. Response-phase header actions apply to the `101`. Body actions don't apply.
- **Message actions.** A new action, after `Script`: `WsMessage{direction, filter: Option<Pattern>, op}`, where `op` is `Drop`, `Replace{find, replace}`, `JsonSet`/`JsonRemove` (the M13 splicer on text messages), `Delay{ms}` or `Breakpoint`. Rewrite scripts gain an optional `onWsMessage(context, message)` that returns the message, a replacement or `null` to drop it. The `.d.ts`, prelude, API surface and completions are updated together.
- **Message breakpoints.** A paused message holds later messages in its direction, so order is kept; the other direction keeps flowing. The Breakpoint Editor edits text, the JSON tree or hex, then Send or Drop. The rule-settings timeout applies, and MCP's `resolve_breakpoint` covers messages.
- **Sending.** `Engine::ws_send(flow, direction, payload)` injects a text or binary message into an open connection (the relay re-frames anyway). The WebSocket panel gets a composer and "Resend" on any recorded message. Replaying a WebSocket flow reconnects with the recorded handshake. MCP gets `send_websocket_message`.
- **Recording.** Messages carry `edited`, `injected` and `dropped` flags (the message types change shape, so the session `VERSION` bumps) and show as badges. The original of an edited message is kept alongside it.
- **Tests.** Against the testkit echo: a handshake header rule, a JSON field rewrite, drop, delay, a breakpoint edit and timeout, injection from the Engine and MCP, and order under load.

**As built (M19).** Where it differs from the above, or adds detail:
- **Model.** `Action::WsMessage { direction: Option<WsDirection>, filter: Option<Pattern>, op: WsOp }` is the last `Action` variant (`None` direction: both ways). `WsOp` is `Drop | Replace { pattern, replacement } | JsonSet { pointer, value, raw } | JsonRemove { pointer } | Delay { ms } | Breakpoint`, internally tagged by `kind` (`pattern`/`replacement` as `ReplaceBody` has them, rather than the plan's `find`/`replace`). The filter is a find pattern: exact text is a substring, a glob matches the whole payload, a regex anywhere. `Action` isn't stored in `FlowDetail` (only `AppliedRule` is), so the action alone wouldn't have needed a session bump; the message flags do. `WsMessage` (the recorded message) appends `edited`, `injected`, `dropped`; `BreakpointInfo` gains `message: Option<HeldMessage { direction, opcode }>`; `MessageEdits` gains `body_base64` (messages only; it wins over `body`); `ScriptPhase` gains `WsMessage` (last). `WsSend { direction, payload, binary }` is what `Engine::ws_send` takes.
- **Session format 6.** Each message in the `ws` column carries the three flags, and a new `originals(flow, idx, data)` table keeps what arrived for edited messages. Versions 1–5 still open (their `ws` column decodes with the old message layout; there is no `originals` table). HAR export leaves dropped messages out (they never went over the wire).
- **Handshakes.** `forward` and the MITM send WebSocket handshakes through `Exchange::run` with `websocket: Some(ClientSide)`. The request phase runs without body buffering; Map Local and mock answers are ignored (a handshake is answered by the upstream or not at all), Block and Respond answer the client with a non-`101`, and a Map Remote or an edited URL naming `http(s)` becomes `ws(s)`. Each handshake dials its own connection (`mitm::open_h1`) except on the MITM's own origin. On `101`, `switch_with_rules` runs the gated response phase on the head only (header actions, `onResponse` without a body, delays; no status change or breakpoint) and records the head as sent.
- **Relay (`proxy/ws.rs`).** Per direction, a reader task queues frames (1,024 ahead) and a processor relays them in order: control frames as they are, data messages through `rules::eval::ws_message_phase` (ops and `onWsMessage` in rule and action order, a drop ends the run, delays add up, the first breakpoint fires after the rewrites). The relay keeps the handshake as the client sent it and re-matches when `Engine::rule_set()` changes (an `Arc` pointer check per message). Rules with only `WsMessage` actions are skipped for plain HTTP requests, so they never send traffic down the rules path. JSON ops skip binary and non-JSON text with a `rewrite_skipped` note; a text message whose bytes stop being UTF-8 goes as binary. Applied rules are recorded once per rule and direction (`Phase::Request` for sent, `Response` for received); script issues are capped at 20 per connection.
- **Breakpoints.** A held message is a `BreakpointInfo` with `message` set, `request` the handshake as sent, and the payload as the body (editable up to 16 MB). `Engine::resolve_breakpoint` refuses `Respond` and head edits for messages and checks `body_base64`; `Abort` drops the message and the connection stays open. Dropping the relay (a side closed for good) removes the pending entry.
- **Sending.** `Engine::ws_send(flow, &WsSend)` and `Engine::ws_close(flow)` go through a registry of open connections (`proxy::ws::Registry`, by live flow id); an injected message joins its direction's queue, so it keeps its place behind a held one, and skips the rules. Closing injects a 1000 close frame each way and counts as the side it speaks for. `Engine::websocket_is_open` and `websocket_original_in` round it out. Replay of a WebSocket flow (`Engine::send` with a `ws(s)://` spec; `request_spec` turns a recorded `http(s)` absolute-form URL back into `ws(s)`) sends a GET with a fresh `Sec-WebSocket-Key` and runs the relay with no client (`ClientSide::Headless`): received messages are recorded, only `sent` can be injected.
- **Scripts.** Rewrite scripts may export only `onWsMessage`. It gets `(context, message)` with `context.phase === "websocket"`; the message has `direction`, a settable `type` and the payload as `body`/`text`/`rawBody` (parsed like a body without headers). Returning the message (or nothing) keeps it with any changes, `null` drops it, a string, bytes or another object replaces it. Payloads over 16 MB pass untouched. No new globals, so `api_surface.txt` is unchanged; the `.d.ts`, prelude, `ENTRY_POINTS`, `MESSAGE_FIELDS` and a "Rewrite WebSocket messages" template are new. The dry run (`test_rule`) applies a rule's message actions to a WebSocket flow's first 50 recorded messages and notes what each would do.
- **MCP.** `send_websocket_message` (Write), `close_websocket` (Destructive); `get_breakpoint` adds `body_base64` for a binary held message; `get_websocket_messages` adds `original`.
- **UI.** The rule editor's "WebSocket message" action (direction, filter, op). The Breakpoint Editor shows a held message with Text, Tree (JSON) and Hex modes and Send or Drop. The WebSocket panel shows Edited, Injected and Dropped badges, a Forwarded/Original toggle for edited messages, and, while a live connection is open, a composer (to server or client, text or hex), Resend (row menu and detail) and Close. The flow menu's Replay reconnects a WebSocket flow. The mock backend keeps odd WebSocket flows open and `?breakpoint=ws` seeds a held message; `e2e/websocket.spec.ts` covers them.
- **Not done (follow-ups).** Running `onWsMessage` in the Scripts window's dry run (`test_script`); per-message rule badges in the panel; editing a message's opcode in the Breakpoint Editor; replaying a WebSocket's recorded client messages after reconnecting; WebSockets over HTTP/2 (extended CONNECT) remain out of scope.

### M20: Offline mocking (`m20-offline-mocking`)
- **Serve from a recording.** A new action, after `Script`: `ServeFrom{source, key, on_miss, pick}`.
  - `source` is a mock library entry: "Mock from session…" on a document tab or saved session copies the session or HAR into `<data dir>/mocks/<uuid>.proxymate`, so the rule survives the original file moving.
  - `key` is method, scheme, host, path and the query compared order-insensitively, optionally plus a body hash.
  - `on_miss` is pass through, 404 or 502. `pick` is the first, last or round-robin of the matching entries.
  - The index is built when rules compile and held in the `CompiledRuleSet`; bodies are read through `SqliteBackend`. It is terminal like `MapLocal`: the first answer (Map Local, a mock script or ServeFrom) wins, and response-phase actions still run.
  - The preset drafts one rule matching the session's hosts. MCP gets a `create_serve_from` sugar tool.
- **Charles import.** Charles JSON (`.chlsj`) and XML (`.chlsx`) sessions open as documents, like HARs. The binary `.chls` format is Java serialisation and is not supported; the import dialog says to export JSON from Charles.
- **OpenAPI import.** OpenAPI 3.0 and 3.1 (JSON or YAML) specs are stored in `app.sqlite`.
  - Flows whose method and path match an operation show its `operationId` and summary in the inspector, with a DSL `op:` term.
  - "Generate mocks" drafts one rule per operation from its examples (or a minimal instance of the response schema) as inline Map Local bodies, disabled until reviewed.
  - Validating responses against the schema is a follow-up.
- **Tests.** Record against the testkit upstream, save, stop the upstream, and check ServeFrom answers byte-identically (misses and round-robin too). Charles fixtures exported from Charles 5 round-trip, and the Petstore spec's rules answer.

### M21: Capture targets and noise (`m21-capture-targets`)
- **Launchers.** An "Intercept" toolbar menu and palette commands start pre-configured targets through a new `pm-platform` `Launcher` trait (tested with a fake `command::Runner`).
  - **Terminal:** Terminal.app or iTerm, with the "Copy terminal env" variables plus `NODE_USE_ENV_PROXY`, `GIT_SSL_CAINFO`, `AWS_CA_BUNDLE`, `DENO_CERT`, `PIP_CERT`, `CARGO_HTTP_CAINFO` and `JAVA_TOOL_OPTIONS` proxy properties.
  - **Chromium browsers** (Chrome, Edge, Brave, Arc): a temporary `--user-data-dir`, `--proxy-server` and `--ignore-certificate-errors-spki-list=<CA SPKI hash>`, so they decrypt without keychain trust.
  - **Firefox:** a temporary profile whose `user.js` sets the proxy and trusts the CA.
  - Launched processes are tracked by pid tree, and their flows show as "<App> (Intercepted)". Docker and JVM attach are follow-ups.
- **Recording allow/block lists.** `RecordingSettings{only, never}` are host globs, checked with a `GlobSet` when a flow begins. Unrecorded traffic is still proxied, the SSL policy and rules still apply, and nothing is teed. The footer counts what was not recorded, and the flow table's "Never record this host" adds to the list.
- **No Caching and Block Cookies.** Toolbar toggles backed by built-in rules like `builtin-throttle`:
  - `builtin-no-cache` removes the conditional request headers and sets `Cache-Control: no-cache` and `Pragma: no-cache`; on responses it sets `no-store` and removes `ETag`, `Last-Modified` and `Expires`.
  - `builtin-no-cookies` removes `Cookie` and `Set-Cookie`.
- **Auto-save.** Settings › Storage can save the live capture every N minutes to a folder as a rotating `.proxymate` (keeping the newest K), off the ingest thread, using M8's background save.
- **Tests.** Launcher command lines are checked against the fake runner. Real launches are a manual checklist in the PR (they need the user's OK). Integration tests cover the recording lists and both built-in rules.

### Later (not scheduled)
- HTTP/3 and QUIC MITM, ws-over-h2 (RFC 8441), h2c.
- Android: the CA in the emulator's system store (writable-system or a tmpfs over `cacerts`, and the APEX store on Android 14+), and Frida unpinning.
- Per-process capture for apps that ignore the system proxy. This needs a Network Extension, which needs Developer ID signing (see "Packaging, signing and updates").
- PAC files and NTLM/Kerberos upstream auth, DoH, gRPC server reflection, protobuf rewrites, OpenAPI validation, Docker interception, saved request collections.
- (From M16) h2c prior knowledge inside a CONNECT (plaintext gRPC through the proxy): `serve_plain` speaks h1 only, so `grpcurl -plaintext` via the proxy fails. A decoded, read-only protobuf view and hex editing in the Breakpoint Editor.

## Critical files (to be created)
- `crates/pm-model/src/lib.rs`: the shared types that drive the TS bindings and the MCP schemas.
- `crates/pm-core/src/proxy/mitm.rs`: CONNECT, the ALPN-mirrored handshake, passthrough and pinning detection.
- `crates/pm-core/src/store/view.rs`: the summary arena, incremental views and tick deltas.
- `crates/pm-core/src/rules/eval.rs`: evaluation order, body policy and breakpoint integration.
- `crates/pm-platform/src/macos/sysproxy.rs`: system proxy backup, guardian and restore.
- `crates/pm-mcp/src/tools.rs`: the MCP tool surface.
- `apps/desktop/ui/src/components/FlowTable/`: the virtualised table and the colour tokens.
- `crates/pm-core/src/io/content.rs` and `apps/desktop/ui/src/components/BodyViewer/`: content detection and the viewer registry.
- `docs/PLAN.md` and `CLAUDE.md`: the plan and conventions that every milestone session reads.

## Verification
- **Automated (cargo test, using pm-testkit upstreams with reqwest/curl as clients):**
  - h1 status codes, chunked bodies, gzip/br, and a 50 MB passthrough with bounded memory.
  - `curl -x http://127.0.0.1:9090 --cacert ca.pem https://localhost:<port> --http2 -v` shows `ALPN: h2`; `--http1.1` also works.
  - Excluded hosts become tunnel flows, and a self-signed upstream produces a flow error.
  - Rules matrix: Map Local serving a file, JsonSet on a gzipped upstream, a breakpoint resolved programmatically, a breakpoint timeout, and an over-cap body streaming with a warning.
  - WS echo (text, binary, ping, close) and incremental SSE.
  - Golden-file tests for HAR and cURL, plus a round trip of a Chrome DevTools HAR.
  - Content-detection unit tests per ContentKind, covering header-less bodies, mislabelled Content-Types and compressed bodies.
- **Body viewers:** a pm-testkit "zoo" endpoint serves one fixture per ContentKind (JSON, nested and 10 MB JSON, XML, HTML, form, multipart with an image part, GraphQL, JS, CSS, PNG, animated GIF, WebP, SVG with an embedded script, MP4, PDF, font, random binary). Playwright snapshots each viewer in dark and light. The SVG and HTML previews must not execute scripts or make network requests.
  - The fixtures and `zoo.json` live in `crates/pm-testkit/fixtures/zoo/`. Playwright runs against the UI's mock backend serving the same fixtures (`?zoo`), in WebKit (the app's engine); the preview-safety checks also run in Chromium.
  - Snapshot baselines are committed per platform. (M10) CI runs the UI job in the Playwright Docker image (`mcr.microsoft.com/playwright:v1.64.1-noble`) with pixel comparison on; missing Linux baselines are written there and come back in the `playwright-report` artifact to be committed.
- **Performance:**
  - Criterion bench: ingest 100k flows in under 2 s, and refilter or sort in under 10 ms.
  - The UI scrolls 100k rows at 60 fps (checked in the Performance panel).
  - A 1-hour soak with `proxymate-cli bench` at 500 rps keeps RSS stable. (M10) `proxymate-cli bench --rps 500 --duration 3600` sends a mix of bodies (JSON, gzip/br, 20 KB and 300 KB binaries, POSTs, errors, the zoo) through its own engine, or through the app with `--proxy 127.0.0.1:9090 --pid <pid>`, and prints rate, latency, failures, RSS, flows and disk as CSV.
- **macOS manual checks:**
  - Safari and Chrome traffic decrypts, with app icons.
  - After `kill -9` of the app, `scutil --proxy` shows the proxy restored within 1 s.
  - Safari in the iOS Simulator decrypts.
  - A physical iPhone decrypts after installing the profile and enabling full trust.
- **MCP:**
  - Use `npx @modelcontextprotocol/inspector` against `http://127.0.0.1:9095/mcp`.
  - Toggling read-only updates the tool list live.
  - After `claude mcp add …`, an agent creates a Map Local and the UI shows the AI badge.
  - Rust tests run an rmcp client against a headless Engine.
- **UI:**
  - Playwright smoke tests with `@tauri-apps/api/mocks`. tauri-driver doesn't support macOS.
  - A manual checklist against the real app.
- **Packaging:** `spctl -a -vv` and `xcrun stapler validate` pass.
