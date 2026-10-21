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

**What's out of v1:** JS scripting, HTTP/3, ws-over-h2 (RFC 8441), and a privileged helper.

**MCP access:** agents get full access by default. A setting switches the server to read-only, and rules created by agents are tagged.

**Delivery:** this is a new GitHub repository. Each milestone below is built in its **own Claude session** and lands as **its own PR** (see "Delivery workflow").

Design cues from Proxyman's dashboard:
- Three panes: a sidebar grouped by Apps and Domains with counts, a dense monospace flow table, and a split Request | Response inspector.
- A row of filter chips: All/HTTP/HTTPS/WebSocket | JSON/Form/XML/JS/CSS/GraphQL/Document/Media/Other.
- Method and status pills, and an orange highlight on the selected row.
- A toolbar status reading "Listening on IP:port", and a footer with Clear / Filter / Auto Select, the row count and throughput.

## Architecture

### Workspace layout
The Cargo workspace uses edition 2024, plus an npm workspace for the UI.
```
crates/pm-model     pure types (serde + specta + schemars): Flow*, Rule/Matcher/Action, FilterSpec, Settings
crates/pm-core      headless Engine: proxy/, tls/, store/, rules/, io/ (har, curl, session) — no Tauri dep
crates/pm-platform  OS traits + cfg impls: sysproxy, trust, procinfo, netif, simctl (macos/ linux/ windows/)
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
- **Bodies.** Bodies of 8 KB or less are inlined. Larger bodies are stored still content-encoded in 256 MB rotating segment files under `~/Library/Application Support/Proxymate/live/<uuid>/`. They are decoded only when viewed.
- **Flow detail.** `FlowDetail` is postcard-encoded into the segments, with a moka LRU in front of them. A flow still in flight is read from its live recorder instead.
- **Live directory lifetime.** Clear deletes the segment files and quit deletes the directory. Each live directory holds an exclusive file lock, so the next launch sweeps directories left behind by a crash or a signal.
- **Caps.**
  - Body capture is capped at 64 MB. Above that the recorded copy is truncated, but forwarding is never affected.
  - Flows are capped at 250k with ring eviction. Pinned flows are kept.
- **Sessions.** A saved session (`.proxymate`) is a single SQLite file. A `BlobBackend` trait lets the same store and view code read live segments, saved sessions and imported HARs.
- **App database.** `app.sqlite` (rusqlite, WAL) holds rules, settings, the SSL include/exclude lists, the LAN client allow-list and the MCP audit log.
  - Until it exists, the SSL lists (include, exclude, bypass, insecure upstream) live in `<data_dir>/ssl.json` (M2).

### Filtering at 100k+ rows (in Rust)
- **Views.** Each `View` holds `{CompiledFilter, SortSpec, indices: Vec<u32>}` and is maintained incrementally as flows arrive and change.
- **Refilter speed.** A full refilter is a linear scan of the summaries (rayon above 50k), with a target under 10 ms.
- **Deep filters.** Header and body predicates run as a cancellable background job that streams matching ids into the view.
- **Filter DSL.** One handwritten parser is shared by the search box, the chips and MCP. Example: `host:*.example.com method:POST status:4xx ct:json app:Safari dur>500 size>1mb -host:*.google.com "text"`.

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
| Content decoding | flate2, brotli and zstd, run synchronously on stored bodies when the inspector asks (decoding is never on the forwarding path; M5 may add async-compression for streaming rewrites) |
| Rule matching | regex (with `size_limit`) and globset |
| Concurrency | arc-swap, parking_lot, rayon |
| QR codes | qrcode |
| Network interfaces | if-addrs |

We build our own engine and use hudsucker only as a reading reference. Its handler model can't express editable breakpoints or ALPN mirroring.

### Connection pipeline
1. **Accept and sniff.**
   - Loopback clients are always allowed. A new LAN IP raises an "Allow?" prompt, and its connections are held for up to 30 s while the user decides.
   - Then peek at the first bytes to choose a path: `CONNECT` goes to the tunnel, an absolute-form URI goes to the forward proxy, and an origin-form request addressed to us goes to the **onboarding server** (`/`, `/cert.pem`, `/cert.crt`, `/proxymate.mobileconfig`).
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
7. **Streaming.** Bodies are tee'd by default, so SSE and LLM token streams stay live in the UI.
8. **Port.** Listen on 9090. If it's taken, try 9091–9099 and show the port in use.
9. **Proxied terminal.** "Copy terminal env" sets `HTTP(S)_PROXY`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and `CURL_CA_BUNDLE`.

### Known limitations (documented in the UI)
- The Raw tab is a reconstruction, not the exact wire bytes.
- TLS 1.0/1.1 upstreams fail, with a clear error.
- h2c is recorded but deferred.
- HTTPS in absolute form (`GET https://…` sent to the proxy) is refused with 501; clients must use CONNECT.
- WebSocket upgrades inside decrypted HTTPS aren't relayed until M6.
- HTTP/1.1 header names are forwarded with their original case, but recorded lower-cased (hyper doesn't expose its case map) and shown in Title-Case.

## Rules engine
- **Data model.** A `Rule` has `{id, name, enabled, order, origin: User|Preset|Agent{client, session, created_at}, matcher, actions[]}`.
- **Matcher.** Matches on methods, scheme, host, port, path, query and header KV matches (each can be negated), client app, and an optional response matcher (status ranges, content-type).
  - Patterns can be Exact, Glob or Regex, each with a case-insensitive flag.
- **Actions:**
  - Headers: `SetHeader`, `RemoveHeader`, `ReplaceHeaderValue`.
  - Query: `SetQuery`, `RemoveQuery`.
  - Body: `ReplaceBody`, `SetBody`, `JsonSet`, `JsonRemove` (by JSON pointer).
  - Status: `SetStatus`.
  - Routing: `MapLocal` (an inline body, a file, or a directory root mapped by path suffix) and `MapRemote`.
  - Flow control: `Breakpoint{request,response}`, `Delay`, `Throttle` (token bucket), `Block` (a status code or a connection reset).
- **Compilation.** Rules compile to a `CompiledRuleSet`, which is swapped in atomically with ArcSwap.
  - A host `RegexSet` prefilters candidate rules.
  - Regexes that agents supply are compiled with a size limit.
  - Each flow snapshots the rule set when it starts.
- **Evaluation order:**
  1. **Request phase.** All matching rules run in `order`; this is not first-match.
     - `Block` stops the flow.
     - Header, query and body rewrites apply in sequence.
     - If several `MapRemote` rules match, the last one wins.
     - `MapLocal` is terminal: it skips the upstream, but response-phase actions still run.
     - The request breakpoint fires **last**, so the user edits exactly what will be sent.
  2. **Response phase.** Gated by the response matcher: rewrites run, then the response breakpoint.
  3. **Recording.** Each flow records which rules applied. That drives the Edited column and the badges.
- **Breakpoints.**
  - `BreakpointManager` keeps a `DashMap<BpId, Pending{snapshot, oneshot::Sender<Resolution>}>`.
  - A resolution is `Continue{edits}`, `Abort` or `Respond`.
  - The timeout defaults to 300 s, then the flow continues unmodified.
  - The UI and MCP can both resolve a breakpoint. The first to resolve wins.
- **Body policy.** By default bodies stream. If any body action or breakpoint applies, the body is buffered up to 32 MB, decoded, transformed, and re-sent identity-encoded with a fresh Content-Length.
  - A body over the cap is forwarded untouched and the flow is marked `rewrite_skipped`.
- **Presets.** "Map Local…", "Breakpoint…" and "Rewrite…" in the flow context menu create pre-filled rules. The rules editor has a "Test against selected flow" dry run, computed in Rust.

## macOS integration (pm-platform)
- **CA trust.**
  - Install the CA into user-domain trust in the login keychain with `SecTrustSettingsSetTrustSettings` via security-framework (fallback: `security add-trusted-cert -r trustRoot -k login.keychain-db`). macOS shows its own password/Touch ID sheet, and no sudo is needed.
  - The wizard verifies trust with a real handshake: a `rustls-platform-verifier` client connects to our own MITM for `verify.proxymate.test`.
  - Trust is re-checked at launch, and the toolbar warns if it's missing.
  - Uninstall removes the trust setting and deletes the cert.
  - For Firefox, the wizard explains `security.enterprise_roots.enabled`.
- **System proxy.**
  - Use `networksetup` per active service for web, secure web and bypass domains.
  - **M3 spike:** check whether this prompts on macOS 27. If it does, fall back to `SCPreferencesCreateWithAuthorization`, which prompts once per launch.
- **Restoring the system proxy.**
  - Before changing anything, write a backup file containing the pid, host and port.
  - Restore on quit, on SIGINT/SIGTERM and from the panic hook.
  - A detached **guardian process** (`proxymate --sysproxy-guardian <pid>`) waits on kqueue `NOTE_EXIT`, so a SIGKILL or crash still restores.
  - At startup, a stale backup whose pid is dead also triggers a restore.
  - A service is only restored if its setting still points at us.
- **Client app detection.**
  - For loopback peers, map the peer port to a pid with libproc (`proc_pidinfo` FD scan, cached, coalesced, and run in spawn_blocking off the hot path).
  - From the pid, find the `.app` bundle to get the name and bundle id. Helper processes map to their parent app.
  - Icons come from `NSWorkspace.iconForFile` via objc2-app-kit and are served through a `pmicon://` URI scheme.
  - Remote devices show their IP plus a nickname from the allow-list.
- **iOS Simulator.**
  - `xcrun simctl list devices booted -j`, then `xcrun simctl keychain <udid> add-root-cert`. "Install to all booted" is one click.
  - The simulator uses the Mac's system proxy, so no extra proxy setup is needed.
- **Linux and Windows equivalents (M11).**
  - Linux: `pkexec update-ca-certificates`/`update-ca-trust` plus NSS `certutil`; proxy via gsettings or kwriteconfig; app detection via netlink sock_diag and `/proc`.
  - Windows: the CurrentUser ROOT store; proxy via the HKCU Internet Settings registry key and `InternetSetOption`; app detection via `GetExtendedTcpTable`; the guardian uses `WaitForSingleObject`.

## MCP server (pm-mcp, rmcp)
- **Transport.**
  - Streamable HTTP on `127.0.0.1:9095/mcp`.
  - Each install gets its own bearer token, and the Origin/Host headers are checked to block DNS rebinding.
  - Settings has a copy button for `claude mcp add --transport http proxymate … --header "Authorization: Bearer …"`.
- **Stdio bridge.** `proxymate mcp-stdio` relays to the HTTP endpoint and launches the app if it isn't running.
- **Headless.** `proxymate-cli serve --mcp` runs everything without the UI, for CI.
- **Read-only mode.** The mutating tools disappear from `tools/list` and the server sends `tools/list_changed`.
  - Every tool carries `readOnlyHint`/`destructiveHint` annotations.
  - Every call goes into the audit log with the client name.
- **Read tools:**
  - `get_status`, `list_flows{filter?, since_id?, limit, cursor, sort}`.
  - `search_flows{query, regex?, scopes}`.
  - `get_flow{id, include}`, `get_flow_body{id, part, decode, format, json_pointer?, offset, max_bytes}`.
  - `get_websocket_messages`, `wait_for_flow{filter, timeout_ms, after_id}`.
  - `export_curl`, `export_har`, `list_rules`, `get_rule`, `list_breakpoints`, `get_breakpoint`.
- **Write tools:**
  - `replay_flow{id, modify?, apply_rules, wait}`, `send_request`.
  - `create_rule`, `update_rule`, `set_rule_enabled`, `delete_rule`, `delete_agent_rules`.
  - Sugar tools: `create_map_local`, `create_breakpoint`, `create_rewrite`.
  - `resolve_breakpoint{id, continue|abort|respond, edits}`, `set_recording`, `clear_flows`.
- **Resources:**
  - `proxymate://status`, `proxymate://rules`.
  - `proxymate://breakpoints`, which is subscribable and notifies on every hit.
  - Templates: `proxymate://flows/{id}`, `…/request/body`, `…/response/body`, `…/ws`.
- **Prompts.** `investigate_failures` and `mock_endpoint`.
- **In the UI.** Agent rules show an "AI · <client>" badge, and there's a "Remove all agent rules" button. The toolbar shows an MCP indicator with the number of connected clients.

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
  - The context menu has copy URL/cURL, Replay, Edit & Replay, Pin, Map Local…, Breakpoint…, Rewrite…, Export HAR and Add to SSL bypass.
- **Detail pane.**
  - A URL bar with method and status pills.
  - Request | Response split, with tabs Header, Query, Cookies, Body, Raw, JSON, Form, Hex, Preview, Timing and TLS. Timing and TLS sit in the Response pane; the URL bar shows the HTTP version.
  - A WebSocket frame panel and an SSE event list.
- **Footer.** Clear, Filter, Auto Select, "n/m rows selected", ↑↓ throughput and deep-search progress.
- **Windows.** Compose, Rules Editor (drag to reorder, MatcherBuilder, ActionListEditor, dry run), Breakpoint Editor (queue plus editable message with Execute/Abort/Respond), Cert Wizard, Device Setup Wizard (Simulator / iPhone / Android / Android emulator, with QR codes and a live "waiting for device…" state), Settings and Session Manager.

### Body viewers and previews
Every body can be viewed as **pretty, syntax-highlighted text**, as a **structured view** or as a **visual preview**, with Raw and Hex always available as fallbacks.

- **Content detection (in Rust, `pm-core/src/io/content.rs`).**
  - Detection uses the Content-Type header first, then magic-byte sniffing (PNG, JPEG, GIF, WebP, AVIF, ICO, PDF, gzip), then a text heuristic (JSON or XML by first non-space byte).
  - The result is a `ContentKind` enum: Json, Xml, Html, FormUrlEncoded, Multipart, GraphQL, Javascript, Css, Text, Image, Svg, Video, Audio, Font, Pdf, Binary.
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
- **URL parts:** scheme dim, host bright, path normal, query dim-accent.
- **Selection:** orange.
- **Theme:** dark first, with light mode in M10. The body viewers already use tokens for both (`--syntax-*`, `--code-*`, `--checker-*`).

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
| M9 | `m9-device-setup` | Onboarding server, QR codes, LAN IPs, client allow-list, Simulator one-click install, iPhone and Android wizards (emulator is a stretch goal) | Simulator Safari decrypts; physical iPhone decrypts |
| M10 | `m10-polish` | Light-mode pass, keyboard navigation, column persistence, eviction, deep search, DSL autocomplete, error UX | 1-hour soak; stable RSS |
| M11 | `m11-macos-release` | Developer ID signing, hardened runtime, notarisation, DMG, updater, release workflow | `spctl` and `stapler validate` pass |
| M12 | `m12-linux-windows` | pm-platform implementations for Linux and Windows, AppImage/deb/rpm, MSI/NSIS, CI matrix | Integration suite green on all three OSes |

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
  - Snapshot baselines are committed for macOS only, so CI on Linux runs the same tests with pixel comparison off. Linux baselines (generated in the Playwright Docker image) are a follow-up.
- **Performance:**
  - Criterion bench: ingest 100k flows in under 2 s, and refilter or sort in under 10 ms.
  - The UI scrolls 100k rows at 60 fps (checked in the Performance panel).
  - A 1-hour soak with `proxymate-cli bench` at 500 rps keeps RSS stable.
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
