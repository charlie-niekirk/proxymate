# Native parity ledger

This ledger lists every user-facing behaviour of the React UI (`apps/desktop/ui/src`) and the Tauri menu bar (`apps/desktop/src-tauri/src/menu.rs`) that the native gpui app (`apps/native`) must match before the Tauri app is deleted. There is one row per behaviour: every command and keyboard shortcut, menu item (the menu bar and every context menu), viewer, dialog, sheet or window and its controls, setting, toolbar, sidebar and footer control, and notable empty, error and loading states.

How it is used:

- Each phase (G1–G16) owns the rows with its number. When a phase ships, it ticks its rows and fills in Evidence: a test name, or the path of a dev-remote screenshot attached to the phase's PR.
- A gap can be signed off as a deviation instead of built. Mark it `≈` and give the reason in Evidence.
- G14 audits the whole ledger: every row is either `☑` with evidence or `≈` with a reason.
- A behaviour found later that is missing here gets a new row with the next free ID in its section. IDs are never reused or renumbered.

Source paths are relative to `apps/desktop/ui/src` unless they start with `src-tauri/` (relative to `apps/desktop`).

## Legend

| Status | Meaning |
|---|---|
| `☐` | Not started |
| `☑` | Done; Evidence names the test or screenshot |
| `≈` | Signed-off deviation; Evidence gives the reason |

## Window & menu bar

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| WM-01 | Transparent overlay title bar with the traffic lights inset at the Tauri position (16, 22) | `src-tauri/tauri.conf.json` | G0 | ☑ | G0: dev-remote screenshots in the G0 PR |
| WM-02 | Sidebar vibrancy behind the transparent window; every pane paints an opaque background except the sidebar | `src-tauri/src/window.rs`, `components/AppShell.tsx` | G0 | ☑ | G0: dev-remote screenshots in the G0 PR |
| WM-03 | Default window 1440×900, minimum 960×600 | `src-tauri/tauri.conf.json` | G0 | ☑ | G0: dev-remote screenshots in the G0 PR |
| WM-04 | Appearance System/Light/Dark: System follows macOS live, the choice persists, and the native window theme (vibrancy, traffic lights) follows it | `lib/theme.ts`, `store/prefs.ts` | G0 | ☑ | G0: View › Appearance and dev remote `theme`; prefs in ui-prefs.json |
| WM-05 | Layout: resizable sidebar (default 220 px, 160–360) beside the main column (min 600 px); flow table over inspector, 58/42, each at least 120 px | `components/AppShell.tsx` | G1 | ☑ | G1: `shell/mod.rs` (sidebar 160–360, main column ≥ 600 px, table 58% over the inspector, each ≥ 120 px); `tests/render.rs` checks both panes; `docs/native/g1/table-dark.png` |
| WM-06 | Proxymate › About Proxymate | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-07 | Proxymate › Check for Updates… (Help › Check for Updates… off macOS) | `src-tauri/src/menu.rs` | G15 | ☐ | |
| WM-08 | Proxymate › Services submenu | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-09 | Proxymate › Hide Proxymate ⌘H, Hide Others ⌥⌘H, Show All | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-10 | Proxymate › Quit Proxymate ⌘Q | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-11 | File › New Request… ⌘N opens Compose | `src-tauri/src/menu.rs`, `hooks/useAppMenu.ts` | G8 | ☐ | |
| WM-12 | File › Open… ⌘O: pick a session, HAR or Charles file and open it in a new tab | `src-tauri/src/menu.rs`, `hooks/useAppMenu.ts` | G2 | ☑ | G2: `features/documents.rs` `open` (File menu ⌘O, palette); dev remote `openDocument` opened a session and a HAR live (`docs/native/g2/document-tabs-suggestions.png`) |
| WM-13 | File › Import HAR… ⇧⌘O: the open dialog filtered to HAR | `src-tauri/src/menu.rs`, `hooks/useAppMenu.ts` | G2 | ≈ | G2: File › Import HAR… ⇧⌘O opens the panel with the prompt "Open a HAR file"; gpui's open panel takes no file-type filter, so other files show and the engine refuses what it can't read |
| WM-14 | File › Save Session… ⌘S: saves every flow of the active tab | `src-tauri/src/menu.rs`, `hooks/useAppMenu.ts` | G2 | ☑ | G2: `features/documents.rs` `save` (⌘S): every flow of the active tab, in the background; live save to a `.proxymate` file with progress (PR) |
| WM-15 | File › Export HAR… ⇧⌘E: exports what the table shows in the active tab | `src-tauri/src/menu.rs`, `hooks/useAppMenu.ts` | G2 | ☑ | G2: `features/documents.rs` `save` (⇧⌘E): the table's view (`FlowSelection::View`); live export to a `.har` file (PR) |
| WM-16 | File › Scripts… ⇧⌘J opens the Scripts window | `src-tauri/src/menu.rs`, `hooks/useAppMenu.ts` | G10 | ☐ | |
| WM-17 | File › Close Window ⌘W | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-18 | Edit › Undo, Redo | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-19 | Edit › Cut, Copy, Paste, Select All | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-20 | View › Enter Full Screen (⌃⌘F) | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-21 | Window › Minimize ⌘M, Zoom | `src-tauri/src/menu.rs` | G0 | ☑ | G0: apps/native/src/menus.rs |
| WM-22 | Right-click where no app menu applies shows nothing (no stray system menu); text fields and selected text keep Copy, Paste, Look Up | `hooks/useNativeMenuPolicy.ts` | G2 | ☑ | G2: gpui shows no menu where the app adds none; text fields keep gpui-kit's own Copy/Cut/Paste menu (no Look Up) |
| WM-23 | Error toasts: what failed as the title, the reason as the description | `lib/actions.ts` `reportError`, `components/ui/sonner.tsx` | G2 | ☑ | G2: `features/clipboard.rs` `report_error` (title, reason as the description); `clipboard::tests::copy_later_copies_what_loads_and_reports_failures` |
| WM-24 | Copy toasts: "Copied <what>" for 1.2 s, one toast replaced by the next; "Couldn't copy" on failure; silent copies for buttons that show their own tick | `lib/clipboard.ts` | G2 | ☑ | G2: `features/clipboard.rs` (one copy toast replaced by the next, 1.2 s, "Couldn't copy", `copy_silent`); `clipboard::tests::copies_toast_and_menus_copy_through_the_action`, `tests/shell.rs` `context_menus_copy_the_right_text` |
| WM-25 | Clipboard helper: copy text, copy text fetched first (copyLater), copy an image as PNG | `lib/clipboard.ts` | G2 | ☑ | G2: `features/clipboard.rs` `copy`, `copy_later` (background load), `copy_image` (PNG) |
| WM-26 | Context-menu framework: copy menus for single elements, one menu per list keyed by row, "Copy ⌘C" for the selection first wherever text is selectable | `components/menus/items.tsx`, `components/menus/useMenuTarget.ts` | G2 | ☑ | G2: `features/context_menu.rs` (`Menu`: copy entries via `CopyText`, one builder per list keyed by row, e.g. `sidebar::menu_for`); `tests/shell.rs` `context_menus_copy_the_right_text`. No selectable plain text in the chrome yet; text fields have their own menu |
| WM-27 | Copy formats: header lines, header blocks, cookies, Set-Cookie, auth tokens, query and form pairs, pairs as JSON, hex, Base64, DSL filter terms | `lib/copyFormats.ts` | G2 | ☑ | G2: `pm-view::copy_formats`, its vitest table as Rust tests |
| WM-28 | No engine: a message instead of the UI when the engine is unreachable | `components/NoEngine.tsx` | G2 | ≈ | The native app runs the engine in-process (`pm_app::start_engine` before the first frame), so there is no unreachable engine to show |

## Toolbar

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| TB-01 | Record button: red square while recording ("Stop recording · traffic is still forwarded · ⌘E"), green play when paused ("Start recording · traffic is forwarded either way · ⌘E") | `components/Toolbar.tsx` | G2 | ☑ | G2: `shell/toolbar.rs` (filled square / play, tooltips with ⌘E); `tests/shell.rs` `the_palette_opens_with_cmd_k_and_runs_a_command` toggles recording with ⌘E |
| TB-02 | Compose (+) button, tooltip "Compose a request · ⌘N" (stub until Compose exists) | `components/Toolbar.tsx` | G2 | ☑ | G2: `shell/toolbar.rs` Compose button and tooltip; `features::chrome::Compose` says "Compose arrives in G8" until G8 replaces the handler |
| TB-03 | Clear (trash) button, tooltip "Clear all flows · ⌘⌫", disabled on a document tab | `components/Toolbar.tsx` | G2 | ☑ | G2: `shell/toolbar.rs` Clear, disabled on a document tab (`docs/native/g2/document-tabs-suggestions.png`) |
| TB-04 | Empty toolbar space drags the window | `components/Toolbar.tsx` | G2 | ☑ | G2: the toolbar is `shell::chrome::drag`; its controls take their own presses |
| TB-05 | Listening pill: dot and text "Starting proxy…", "Listening on <addr>", "Listening on <addr> · paused", "Proxy not running: <error>" | `components/Toolbar.tsx` `ListeningStatus` | G2 | ☑ | G2: `toolbar::listening` and its pill; `toolbar::tests::a_failed_proxy_offers_its_error` |
| TB-06 | Listening pill click opens Device setup (Settings › Proxy when the proxy failed); tooltips "Device setup · connect a phone, simulator or emulator" or the error with "Click to change the port." | `components/Toolbar.tsx` `ListeningStatus` | G2 | ☑ | G2: the pill's click and tooltips (`shell/toolbar.rs`); Device setup and Settings › Proxy are toasts until G13 and G12 replace `OpenDeviceSetup` / `OpenSettings` |
| TB-07 | Pill menu: Copy address, Copy curl example, Copy SOCKS5 address | `components/Toolbar.tsx` `ListeningStatus` | G2 | ☑ | G2: `toolbar::listening_menu`; `toolbar::tests::the_pill_menu_copies_every_address`, `tests/shell.rs` `context_menus_copy_the_right_text`; live dev remote `contextMenu: listening` (PR) |
| TB-08 | Pill menu: Copy reverse proxy <addr> → <target>, one per reverse listener | `components/Toolbar.tsx` `ListeningStatus` | G2 | ☑ | G2: `toolbar::tests::the_pill_menu_copies_every_address` (reverse TLS listener) |
| TB-09 | Pill menu: Copy terminal environment; Copy error when the proxy failed | `components/Toolbar.tsx` `ListeningStatus` | G2 | ☑ | G2: `toolbar::listening_menu` (terminal environment, error); `toolbar::tests::a_failed_proxy_offers_its_error` |
| TB-10 | Pill menu: Device setup…, Proxy settings…, Network settings… | `components/Toolbar.tsx` `ListeningStatus` | G2 | ☑ | G2: the items are there (`toolbar::listening_menu`); their windows arrive with G12 and G13, which replace the handlers |
| TB-11 | "N devices waiting" button for LAN devices whose Allow prompt was dismissed; click reopens the prompt | `components/Toolbar.tsx` `WaitingDevices` | G13 | ☐ | |
| TB-12 | Intercept button (crosshair) with the count of running intercepted apps, tooltip "Intercept: a terminal or browser that goes through Proxymate" | `components/InterceptMenu.tsx` | G13 | ☐ | |
| TB-13 | Rules button, tooltip "Rules: Map Local, rewrites, breakpoints, Map Remote, block" | `components/Toolbar.tsx` `RulesButton` | G7 | ☐ | |
| TB-14 | Scripts button, tooltip "Scripts: decode bodies, rewrite and mock with JavaScript" | `components/Toolbar.tsx` `ScriptsButton` | G10 | ☐ | |
| TB-15 | No Caching toggle: icon only when off, "No Caching" label and warning colour when on, tooltips for both states | `components/InterceptMenu.tsx` `NoiseToggles` | G7 | ☐ | |
| TB-16 | Block Cookies toggle: same pattern as No Caching | `components/InterceptMenu.tsx` `NoiseToggles` | G7 | ☐ | |
| TB-17 | Network conditions (gauge) button: shows the active throttle preset's name in warning colour | `components/Rules/ThrottleMenu.tsx` | G7 | ☐ | |
| TB-18 | Breakpoints badge: paused count in a pill, tooltip "N paused at a breakpoint" or "Breakpoints · none paused"; opens the Breakpoint Editor | `components/Breakpoints/BreakpointBadge.tsx` | G7 | ☐ | |
| TB-19 | "Certificate not trusted" warning button while listening and the CA isn't verified (keychain supported); opens the certificate wizard | `components/Toolbar.tsx` `TrustWarning` | G13 | ☐ | |
| TB-20 | System proxy (globe) button, green while on | `components/SystemProxyMenu.tsx` | G13 | ☐ | |
| TB-21 | Certificate (shield) button: alert shield while untrusted | `components/CertificateMenu.tsx` | G13 | ☐ | |
| TB-22 | MCP indicator: robot icon, connected-client count, colour by state (off, error, serving, clients), tooltip per state; opens Settings › MCP | `components/Settings/McpIndicator.tsx` | G13 | ☐ | |
| TB-23 | Settings (gear) button, tooltip "Settings · ⌘," | `components/Toolbar.tsx` `SettingsButton` | G12 | ☐ | |

## Sidebar

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SB-01 | 52 px strip under the traffic lights drags the window | `components/Sidebar.tsx` | G2 | ☑ | G2: `shell/sidebar.rs` `sidebar-drag` (`chrome::drag`, 52 px) |
| SB-02 | Collapsible section headers: Favorites, Pinned, Saved, All, Apps, Domains | `components/Sidebar.tsx` `Section` | G2 | ☑ | G2: `Sidebar::section` (role Button, `aria_expanded`); `tests/render.rs` renders every section |
| SB-03 | Favorites: "No favorites yet" placeholder | `components/Sidebar.tsx` | G2 | ☑ | G2: `shell/sidebar.rs` (Favorites placeholder) |
| SB-04 | Pinned: "Pinned flows" with count toggles the `is:pinned` filter; "No pinned flows" when none or on a document tab | `components/Sidebar.tsx` `PinnedItem` | G2 | ☑ | G2: `shell/sidebar.rs` Pinned row (`ToggleText is:pinned`, storage `pinned`); `tests/render.rs` shows "Pinned flows 3" |
| SB-05 | Saved: "Live capture" item switches to the live tab | `components/Sidebar.tsx` `SavedDocuments` | G2 | ☑ | G2: `tests/shell.rs` `chips_and_sidebar_rows_filter` (Saved and tab rows switch stores) |
| SB-06 | Saved: one item per open document (session or HAR icon, name, flow count, path tooltip); click shows it | `components/Sidebar.tsx` `SavedDocuments` | G2 | ☑ | G2: `shell/sidebar.rs` (session/HAR icons, name, count, path tooltip); `tests/shell.rs` `chips_and_sidebar_rows_filter` clicks `saved-1` |
| SB-07 | Saved document menu: Copy name, Copy path, Close <name> | `components/Sidebar.tsx` `SavedDocuments` | G2 | ☑ | G2: `sidebar::document_menu`; `sidebar::tests::document_menus_copy_and_close`, `tests/shell.rs` `context_menus_copy_the_right_text` |
| SB-08 | Saved: "Open session or HAR…" item | `components/Sidebar.tsx` `SavedDocuments` | G2 | ☑ | G2: `shell/sidebar.rs` `saved-open` (File › Open…) |
| SB-09 | All: "All traffic" with total count clears the app and host scope | `components/Sidebar.tsx` | G2 | ☑ | G2: `tests/shell.rs` `chips_and_sidebar_rows_filter` (`sidebar-all` clears the scope) |
| SB-10 | Apps: one row per client app (icon, name, count); click toggles the app scope; "No apps yet" or "No matching apps" | `components/Sidebar.tsx` `AppItem` | G2 | ☑ | G2: `shell/sidebar.rs` apps (icons from `Engine::app_icon`, initial otherwise; counts; empty states); `tests/shell.rs` `context_menus_copy_the_right_text` ("Show only Safari" scopes the table); live (`docs/native/g2/100k-filtered.png`) |
| SB-11 | App row shows a shield-off badge when its HTTPS is never decrypted | `components/Sidebar.tsx` `AppItem` | G2 | ☑ | G2: `shell/sidebar.rs` ShieldOff badge from `SslSettings.exclude_apps` (by bundle id, else name) |
| SB-12 | App menu: Show only <app> / Show all traffic | `components/Sidebar.tsx` `AppItem` | G2 | ☑ | G2: `sidebar::app_menu`; `sidebar::tests::app_menus_copy_the_name_and_bundle_id` |
| SB-13 | App menu: Copy name, Copy bundle id | `components/Sidebar.tsx` `AppItem` | G2 | ☑ | G2: `sidebar::tests::app_menus_copy_the_name_and_bundle_id`, `tests/shell.rs` `context_menus_copy_the_right_text` |
| SB-14 | App menu: "Don't decrypt HTTPS from <app>" checkbox (by bundle id, else name) | `components/Sidebar.tsx` `AppItem` | G2 | ☑ | G2: `sidebar::app_menu` check item → `Engine::set_app_ssl_excluded` (`workspace.rs`) |
| SB-15 | Domains: registrable domains with counts, expandable to their hosts; a domain row scopes to `*.domain`, a host row to the host | `components/Sidebar.tsx` `DomainItem` | G2 | ☑ | G2: `shell/sidebar.rs` domains (chevron expands; `*.domain` / host scopes); `tests/shell.rs` `chips_and_sidebar_rows_filter` |
| SB-16 | Domains: "No domains yet" or "No matching domains"; a filter expands every match | `components/Sidebar.tsx` | G2 | ☑ | G2: `shell/sidebar.rs` (empty states, a filter opens every match); `tests/shell.rs` `cmd_l_filters_the_sidebar` |
| SB-17 | Domain and host menu: Show only <scope> / Show all traffic | `components/Sidebar.tsx` `HostMenu` | G2 | ☑ | G2: `sidebar::host_menu`; `sidebar::tests::host_menus_copy_the_host_and_the_domain_pattern` |
| SB-18 | Domain and host menu: Copy <host>, Copy <*.domain> | `components/Sidebar.tsx` `HostMenu` | G2 | ☑ | G2: `sidebar::tests::host_menus_copy_the_host_and_the_domain_pattern`, `tests/shell.rs` `context_menus_copy_the_right_text`; live dev remote `contextMenu: sidebar:domain:127.0.0.1` (PR) |
| SB-19 | Domain and host menu: Add <host> to SSL bypass | `components/Sidebar.tsx` `HostMenu` | G2 | ☑ | G2: `sidebar::host_menu` → `Engine::add_ssl_bypass` (`workspace.rs`) |
| SB-20 | Domain and host menu: "Accept <host>'s certificate unverified" checkbox | `components/Sidebar.tsx` `HostMenu` | G2 | ☑ | G2: `sidebar::host_menu` check item → `Engine::set_insecure_upstream` (`workspace.rs`) |
| SB-21 | "Filter apps and domains" box at the bottom (⌘L); Escape clears it | `components/Sidebar.tsx` | G2 | ☑ | G2: `shell/sidebar.rs` filter box (Escape clears: `clean_on_escape`), ⌘L (`navigate.rs`); `tests/shell.rs` `cmd_l_filters_the_sidebar` |
| SB-22 | Long app and host names truncate; the tooltip has "<name> · N flows" | `components/Sidebar.tsx` `Item` | G2 | ☑ | G2: `shell/sidebar.rs` rows truncate; tooltip "<name> · N flows" |
| SB-23 | Counts update live from the engine's sidebar snapshot (at most every 250 ms) | `store/sidebar.ts` | G2 | ☑ | G2: `bridge::sidebar` (`subscribe_sidebar`, one subscription for every window); the table sets `set_sidebar_store` with its view; live ({S}live-traffic.png`) |

## Filter bar

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| FB-01 | Protocol chips, one at a time: All, HTTP, HTTPS, WebSocket | `components/FilterBar.tsx` | G2 | ☑ | G2: `shell/filter_bar.rs` protocol chips; `tests/shell.rs` `chips_and_sidebar_rows_filter`; `tests/render.rs` checks the on chip |
| FB-02 | Content chips: All (clears), JSON, Form, XML, JS, CSS, GraphQL, Document, Media, Other (several at once) | `components/FilterBar.tsx` | G2 | ☑ | G2: `shell/filter_bar.rs` content chips; `tests/shell.rs` `chips_and_sidebar_rows_filter` (several at once, All clears) |
| FB-03 | Search box with the DSL, placeholder "Filter: text, host:*.example.com status:4xx body:token…"; typing is debounced, chips apply at once | `components/FilterBar.tsx` `SearchBox` | G2 | ☑ | G2: `shell/filter_bar.rs` (placeholder; text debounced 120 ms by `FlowTable::apply_filter`, chips at once); `tests/shell.rs` `the_search_box_autocompletes_and_hands_back_to_the_table` |
| FB-04 | Autocomplete of keys with their help: host, path, method, status, ct, grpc, app, scheme, state, is, label, comment, op, header, reqheader, resheader, body, reqbody, resbody, dur, size, reqsize, id | `lib/dsl.ts` | G2 | ☑ | G2: `pm-view::dsl` (`DSL_KEYS`, numeric keys with operators), its vitest table as Rust tests |
| FB-05 | Value suggestions: known hosts and `*.domain`s, apps, and fixed lists for method, status, ct, grpc, scheme, state, is and label | `lib/dsl.ts` | G2 | ☑ | G2: `pm-view::dsl` (`DslContext::from_sidebar`); `tests/shell.rs` suggests `*.example.com` and the host; live (`docs/native/g2/document-tabs-suggestions.png`) |
| FB-06 | Key aliases: domain, client, type, content, proto, respbody, respheader, note, operation | `lib/dsl.ts` | G2 | ☑ | G2: `pm-view::dsl::tests::aliases_suggest_their_keys_values` |
| FB-07 | Suggestion keys: ↓ opens, ↑/↓ pick, Tab or ↩ accept, Escape closes; Escape or ↩ with no list goes to the flow table | `components/FilterBar.tsx` `SearchBox` | G2 | ☑ | G2: `FilterBar::on_key` (keystroke interceptor); `tests/shell.rs` `the_search_box_autocompletes_and_hands_back_to_the_table` |
| FB-08 | Clear search (×) button | `components/FilterBar.tsx` `SearchBox` | G2 | ☑ | G2: `shell/filter_bar.rs` `clear-search` |
| FB-09 | Deep-search icon in the box while the query has deep terms | `components/FilterBar.tsx` `SearchBox` | G2 | ☑ | G2: `shell/filter_bar.rs` ScanSearch icon while `dsl::has_deep_terms` |
| FB-10 | Deep-search percentage inside the box while it runs, tooltip "Searching headers and bodies: matches appear as they are found" | `components/FilterBar.tsx` `DeepBadge` | G2 | ☑ | G2: `shell/filter_bar.rs` `deep-badge` with its tooltip |

## Flow table

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| FT-01 | Virtualised grid at a fixed 22 px row height; 100k rows scroll at 60 fps | `components/FlowTable/FlowTable.tsx` | G1 | ☑ | G1: perf probe, 100k rows: 120 fps in all three scroll runs, p95 8.4 ms, no frame over 20 ms (PR) |
| FT-02 | Only the visible window plus read-ahead is fetched; rows not loaded yet show "…" | `components/FlowTable/FlowRow.tsx`, `lib/rowCache.ts` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::loads_only_the_visible_window_in_capped_fetches`, `pm-view` `row_cache::tests::*`; "…" placeholder row in `flow_grid/row.rs` |
| FT-03 | Column # (Number): row number, pin icon on pinned flows | `components/FlowTable/FlowRow.tsx` `RowCell` | G1 | ☑ | G1: `flow_grid/row.rs` `cell`; pins in `docs/native/g1/offscreen-light.png` |
| FT-04 | State column: coloured dot per state, pulsing while active, faded once completed | `components/FlowTable/cells.tsx` `StateDot` | G1 | ☑ | G1: `flow_grid/row.rs` (`pulse` while active, 40% once completed) |
| FT-05 | URL column: scheme, host, port, path and query coloured; Replay, Repeat or AI sent badge; GraphQL operation badge | `components/FlowTable/cells.tsx` `UrlText` | G1 | ☑ | G1: `flow_grid::row::tests::a_row_is_formatted_once_with_its_url_runs`; badges in `docs/native/g1/offscreen-dark.png` |
| FT-06 | Client column: app icon and name, or "—" | `components/FlowTable/FlowRow.tsx` `RowCell` | G1 | ☑ | G1: `flow_grid/icons.rs` (`Engine::app_icon` bytes, loaded off the foreground thread, cached as gpui images); synthetic clients have no icon |
| FT-07 | Method column: coloured method pill | `components/FlowTable/cells.tsx` `MethodPill` | G1 | ☑ | G1: `flow_grid/row.rs` `method_colors`; `docs/native/g1/offscreen-light.png` |
| FT-08 | Status column: pill coloured by class, outlined when failed, pulsing while active, gRPC status by name with "gRPC … · HTTP …" tooltip | `components/FlowTable/cells.tsx` `StatusPill` | G1 | ☑ | G1: `pm-view` `flow::tests::grpc_statuses_read_by_name`; outline, pulse and gRPC tooltip in `flow_grid/row.rs` |
| FT-09 | Time, Duration, Request size and Response size columns | `components/FlowTable/FlowRow.tsx` `RowCell` | G1 | ☑ | G1: `pm-view` `flow::tests::sizes_and_durations_read_compactly`, `times_are_local_with_milliseconds` |
| FT-10 | Proto column: short HTTP version, full version in the tooltip | `components/FlowTable/FlowRow.tsx` `RowCell` | G1 | ☑ | G1: `flow_grid/row.rs` (label and tooltip) |
| FT-11 | SSL column: lock (decrypted), open lock (passed through), shield alert (likely pinned), with tooltips | `components/FlowTable/cells.tsx` `SslIcon` | G1 | ☑ | G1: `flow_grid/row.rs` `ssl_cell` with `pm-view` `flow::ssl_title` tooltips |
| FT-12 | Notes column: label dot, comment icon (comment in the tooltip), pencil when a rule changed the flow | `components/FlowTable/FlowRow.tsx` `RowCell` | G1 | ☑ | G1: `flow_grid/row.rs` (label dot, comment icon with the comment as tooltip, pencil) |
| FT-13 | Tools column: "…" button on hover opens the row menu under it | `components/FlowTable/FlowRow.tsx` `openRowMenu` | G1 | ☐ | G1 draws the column; its "…" opens the row menu, which G3 builds |
| FT-14 | Header columns show the header's value or "—", with the value as tooltip | `components/FlowTable/FlowRow.tsx`, `components/FlowTable/columns.ts` | G11 | ☐ | |
| FT-15 | Alternating row shading, hover, selection colours; the focused row of a multi-selection is outlined | `components/FlowTable/FlowRow.tsx` | G1 | ☑ | G1: `tests/render.rs` checks row, odd-row and selection pixels in both themes; focused-row outline in `docs/native/g1/table-light-multi-header-columns.png` |
| FT-16 | Click a sortable title (#, URL, Method, Status, Duration, Request, Response, header columns) to sort; again to reverse; arrow on the sorted column | `components/FlowTable/Header.tsx` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::sorting_resizing_and_hiding_change_the_prefs`, `pm-view` `columns::tests::sorting_by_a_column_toggles_its_direction` |
| FT-17 | Drag a column's right edge to resize (to 1600 px); double-click restores the default width | `components/FlowTable/Header.tsx` `ResizeHandle` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::dragging_a_column_edge_resizes_it_and_a_double_click_resets_it` |
| FT-18 | Focused resize handle: ←/→ by 8 px (⇧ 32), Home to the minimum, End to the maximum, ↩ to the default | `components/FlowTable/keys.ts` `resizeKey` | G1 | ☑ | G1: `pm-view` `keys::tests::resize_*`; `pm-widgets` `flow_grid::tests::sorting_resizing_and_hiding_change_the_prefs` (keys on the focused edge) |
| FT-19 | Drag a header onto another to reorder, with a drop indicator | `components/FlowTable/Header.tsx` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::dragging_a_header_onto_another_reorders` (drop indicator: `drag_over` in `header.rs`) |
| FT-20 | Column menu "Columns": a checkbox per hideable column (URL can't be hidden) and per header column | `components/FlowTable/Header.tsx` | G1 | ☑ | G1: native menu in `header.rs` `open_column_menu`; its actions in `pm-widgets` `flow_grid::tests::sorting_resizing_and_hiding_change_the_prefs` |
| FT-21 | Column menu: Add header column… (disabled at 8 header columns) | `components/FlowTable/Header.tsx` | G11 | ☐ | |
| FT-22 | Column menu: Remove "<name>" column, on a header column | `components/FlowTable/Header.tsx` | G11 | ☐ | |
| FT-23 | Column menu: Reset columns | `components/FlowTable/Header.tsx` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::sorting_resizing_and_hiding_change_the_prefs`, `pm-view` `columns::tests::reset_keeps_header_columns_before_tools` |
| FT-24 | Column order, widths, hidden columns and the sort persist across launches | `store/prefs.ts` | G1 | ☑ | G1: `ui-prefs.json`, `prefs::tests::prefs_round_trip_and_tolerate_unknown_or_missing_fields`, `pm-view` `columns::tests::saved_prefs_merge_with_new_and_removed_columns` |
| FT-25 | Click selects; ⌘-click toggles a flow; ⇧-click selects a range (the whole view becomes a view selection, not ids) | `components/FlowTable/FlowTable.tsx` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::click_cmd_click_and_shift_click_select_by_id`, `a_shift_range_beyond_the_loaded_rows_is_fetched_in_steps`; `pm-view` `selection::tests::*` |
| FT-26 | Keys ↑/↓, PgUp/PgDn, Home/End move the selection; with ⇧ they extend it | `components/FlowTable/keys.ts` `tableKey` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::arrows_pages_and_home_end_move_the_selection`, `shift_arrows_extend_and_escape_collapses`; `pm-view` `keys::tests::*` |
| FT-27 | ↩ focuses the inspector; Escape in a multi-selection collapses to the focused flow | `components/FlowTable/keys.ts` `tableKey` | G1 | ☐ | G1: Escape in `pm-widgets` `flow_grid::tests::shift_arrows_extend_and_escape_collapses`; ↩ raises `FlowGridEvent::Inspect` (`flow_grid::tests::cmd_a_enter_and_backspace`), and G3 moves the focus into its inspector |
| FT-28 | A keyboard move onto a row not loaded yet selects it once it loads | `components/FlowTable/FlowTable.tsx` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::arrows_pages_and_home_end_move_the_selection` (End); a ⇧-move waits too |
| FT-29 | Right-click inside a multi-selection keeps it (bulk menu); elsewhere it selects the row first; outside any row there is no menu | `components/FlowTable/FlowTable.tsx` | G1 | ☐ | G1: the selection half (`FlowGrid::press_row`); the row menu is G3, the bulk menu G11 |
| FT-30 | Auto Select: scrolls to and selects the newest flow as traffic arrives | `components/FlowTable/FlowTable.tsx` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::auto_select_follows_the_newest_flow` |
| FT-31 | Empty state "Waiting for traffic" with the `curl -x http://<proxy> http://example.com/` example and its "Copy command" menu | `components/FlowTable/EmptyState.tsx` | G1 | ☑ | G2: the empty state's command opens `toolbar::empty_state_menu` on right-click (`FlowGridEvent::EmptyStateMenu`); `tests/shell.rs` `the_empty_state_copies_its_command` |
| FT-32 | Empty states "No flows match the current filter" and "Searching headers and bodies…" | `components/FlowTable/EmptyState.tsx` | G1 | ☑ | G1: `features/flow_table.rs` `update_overlay` (`Overlay::NoMatch`, `Overlay::Searching`) |
| FT-33 | View error: "Couldn't open the flow list", the error, Try again | `components/FlowTable/EmptyState.tsx` `ViewError` | G1 | ☑ | G1: `Overlay::Error` with a gpui-kit "Try again" button (`FlowGridEvent::Retry` reopens the view) |
| FT-34 | Accessible grid: header is row 1, the grid keeps focus and names the focused row, multi-selectable | `components/FlowTable/FlowTable.tsx` | G1 | ☐ | G1: grid, row and column-header roles, header as row 1, the focused row is the active descendant; gpui 0.3 has no multiselectable setter (G14 audits) |
| FT-35 | The inspector follows the focused flow | `components/FlowTable/FlowTable.tsx`, `store/ui.ts` | G1 | ☑ | G1: the inspector placeholder follows the focused flow (`docs/native/g1/table-dark.png`); G3 fills it |

### Flow row menu

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| FM-01 | Copy <column> <value>: the cell right-clicked (Number, State, App, Method, Status, Time, Duration, sizes, Protocol, Comment, header column) | `components/FlowTable/FlowContextMenu.tsx` `cellText` | G3 | ☐ | |
| FM-02 | Copy URL | `components/FlowTable/FlowContextMenu.tsx` | G3 | ☐ | |
| FM-03 | Copy as cURL (disabled for CONNECT) | `components/FlowTable/FlowContextMenu.tsx` | G3 | ☐ | |
| FM-04 | Copy as Code ▸ JavaScript (fetch), Python (requests), HTTPie, Go (net/http), Swift (URLSession), Kotlin (OkHttp), PowerShell (disabled for CONNECT and WebSockets) | `components/FlowTable/FlowContextMenu.tsx`, `lib/flow.ts` | G11 | ☐ | |
| FM-05 | Copy ▸ Host, Path | `components/FlowTable/FlowContextMenu.tsx` | G3 | ☐ | |
| FM-06 | Copy ▸ Request headers, Response headers ("No response yet" error) | `components/FlowTable/FlowContextMenu.tsx` `headersOf` | G3 | ☐ | |
| FM-07 | Copy ▸ Request body, Response body (binary bodies explain they need Base64 from the Body tab) | `components/FlowTable/FlowContextMenu.tsx` `bodyOf` | G3 | ☐ | |
| FM-08 | Copy ▸ Raw request, Raw response | `components/FlowTable/FlowContextMenu.tsx` `rawOf` | G3 | ☐ | |
| FM-09 | Filter by <header> on a header column cell (adds a `reqheader:`/`resheader:` term) | `components/FlowTable/FlowContextMenu.tsx` | G11 | ☐ | |
| FM-10 | Filter by ▸ Host, Method, Status, App (adds a term to the search box) | `components/menus/filter.tsx` `FilterBySub` | G3 | ☐ | |
| FM-11 | Replay ⌘R (disabled for CONNECT and WebSockets) | `components/FlowTable/FlowContextMenu.tsx` | G8 | ☐ | |
| FM-12 | Edit & Replay… | `components/FlowTable/FlowContextMenu.tsx` | G8 | ☐ | |
| FM-13 | Repeat… ⌥⌘R | `components/FlowTable/FlowContextMenu.tsx` | G11 | ☐ | |
| FM-14 | Pin / Unpin | `components/FlowTable/FlowContextMenu.tsx` | G3 | ☐ | |
| FM-15 | Label ▸ Red, Orange, Yellow, Green, Blue, Purple (✓ on the current), No label | `components/FlowTable/FlowContextMenu.tsx` `LabelSub` | G11 | ☐ | |
| FM-16 | Mark for Compare | `components/FlowTable/FlowContextMenu.tsx` | G11 | ☐ | |
| FM-17 | Compare with Marked (#N, ", other tab" across tabs) | `components/FlowTable/FlowContextMenu.tsx` | G11 | ☐ | |
| FM-18 | Comment… / Edit Comment… | `components/FlowTable/FlowContextMenu.tsx` | G11 | ☐ | |
| FM-19 | Map Local… (preset, opens an unsaved rule) | `components/FlowTable/FlowContextMenu.tsx` | G7 | ☐ | |
| FM-20 | Breakpoint… (preset, opens an unsaved rule) | `components/FlowTable/FlowContextMenu.tsx` | G7 | ☐ | |
| FM-21 | Rewrite… | `components/FlowTable/FlowContextMenu.tsx` | G9 | ☐ | |
| FM-22 | Script ▸ New view script…, New rewrite script…, New mock script… | `components/FlowTable/FlowContextMenu.tsx` | G10 | ☐ | |
| FM-23 | Export HAR… (this flow) | `components/FlowTable/FlowContextMenu.tsx` | G3 | ☐ | |
| FM-24 | Add <host> to SSL bypass (HTTPS flows only) | `components/FlowTable/FlowContextMenu.tsx` | G3 | ☐ | |
| FM-25 | Never record <host>, then a toast "Not recording <host>" with a Settings action | `components/FlowTable/FlowContextMenu.tsx`, `lib/actions.ts` `neverRecordHost` | G3 | ☐ | |
| FM-26 | Delete ⌫ (disabled while the flow is active or paused) | `components/FlowTable/FlowContextMenu.tsx` | G3 | ☐ | |
| FM-27 | Bulk menu header "N flows selected" | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu` | G11 | ☐ | |
| FM-28 | Bulk: Copy URLs, Copy as cURL | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu` | G11 | ☐ | |
| FM-29 | Bulk: Replay in Order | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu` | G8 | ☐ | |
| FM-30 | Bulk: Compare (enabled with exactly two) | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu` | G11 | ☐ | |
| FM-31 | Bulk: Pin, Unpin | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu` | G11 | ☐ | |
| FM-32 | Bulk: Label ▸ | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu` | G11 | ☐ | |
| FM-33 | Bulk: Export HAR…, Save Session… (the selection) | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu` | G11 | ☐ | |
| FM-34 | Bulk: Delete N Flows ⌫, toast "Deleted N flows" | `components/FlowTable/FlowContextMenu.tsx` `BulkMenu`, `lib/actions.ts` | G11 | ☐ | |

## Footer

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| FO-01 | Clear button (disabled on a document tab), tooltip "Clear all flows · ⌘⌫" | `components/Footer.tsx` | G2 | ☑ | G2: `shell/footer.rs` `footer-clear` (disabled on a document tab) |
| FO-02 | Filter button focuses the search box, tooltip "Search flows · ⌘F" | `components/Footer.tsx` | G2 | ☑ | G2: `shell/footer.rs` `footer-filter` → `shell::focus_search` |
| FO-03 | Auto Select toggle | `components/Footer.tsx` | G1 | ☑ | G2: `shell/footer.rs` `auto-select` (role Switch) → `capture::toggle_auto_select` |
| FO-04 | Row count "N of M rows", "1 / " with a flow selected, "N selected ·" with several | `components/Footer.tsx` | G2 | ☑ | G2: `footer::row_count`; `footer::tests::counts_read_like_the_tauri_footer`; live "1 / 26,583 of 100,000 rows" (`docs/native/g2/100k-filtered.png`) |
| FO-05 | Deep search: "Searching bodies · checked of total (p%) · N found", then "Searched N flows · N found" | `components/Footer.tsx` `DeepStatus` | G2 | ☑ | G2: `footer::deep_text`; `footer::tests::counts_read_like_the_tauri_footer` |
| FO-06 | Save and export status: "<label> · p%", "Saved N flows to <name>" (4 s), "<label> failed: <error>"; menu Copy path, Copy file name, Copy error, Dismiss | `components/Footer.tsx` `SaveStatus` | G2 | ☑ | G2: `footer::save_text`, `footer::save_menu`; `footer::tests::save_status_and_its_menu`, `tests/shell.rs` `context_menus_copy_the_right_text` |
| FO-07 | "Not recording bodies" (a capture write failed) opens Settings › Storage | `components/Footer.tsx` `StorageWarning` | G2 | ☑ | G2: `shell/footer.rs` `storage-error` (Settings › Storage is a toast until G12) |
| FO-08 | "N not recorded" (recording lists) opens Settings › Storage | `components/Footer.tsx` `Unrecorded` | G2 | ☑ | G2: `shell/footer.rs` `unrecorded-count`; `tests/render.rs` shows "12 not recorded" |
| FO-09 | Filter timing "filter X ms · Y ms end to end" | `components/Footer.tsx` | G2 | ☑ | G2: `shell/footer.rs` `filter-timing`; live "filter 2.2 ms · 9 ms end to end" (`docs/native/g2/100k-filtered.png`) |
| FO-10 | Throughput "↑ … ↓ …" per second ("↑ — ↓ —" before the first status) | `components/Footer.tsx` `Throughput` | G2 | ☑ | G2: `shell/footer.rs` throughput, sampled every second from `StorageStatus` |

## Document tabs

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| DT-01 | Tab strip shown only while a document is open; "Live" first with a green dot | `components/DocumentTabs.tsx` | G2 | ☑ | G2: `shell/tabs.rs` (only while a document is open); `tests/render.rs` |
| DT-02 | Document tab: session or HAR icon, name, flow count, path tooltip; click switches the table and inspector to it | `components/DocumentTabs.tsx` | G2 | ☑ | G2: `shell/tabs.rs`; `tests/shell.rs` `chips_and_sidebar_rows_filter` (`tab-0`); live (`docs/native/g2/document-tabs-suggestions.png`) |
| DT-03 | Close (×) button on each document tab | `components/DocumentTabs.tsx` | G2 | ☑ | G2: `shell/tabs.rs` `close-<store>` |
| DT-04 | Tab menu: Copy name, Copy path | `components/DocumentTabs.tsx` `TabMenu` | G2 | ☑ | G2: `tabs::tab_menu`; `tabs::tests::tab_menus_copy_save_export_and_close`, `tests/shell.rs` `context_menus_copy_the_right_text` |
| DT-05 | Tab menu: Save session…, Export HAR… (switches to that tab first) | `components/DocumentTabs.tsx` `TabMenu` | G2 | ☑ | G2: `tabs::tab_menu` (`SaveStore`/`ExportStore` switch to the tab first) |
| DT-06 | Tab menu: Mock from session… | `components/DocumentTabs.tsx` `TabMenu` | G12 | ☐ | |
| DT-07 | Tab menu: Close (documents only) | `components/DocumentTabs.tsx` `TabMenu` | G2 | ☑ | G2: `tabs::tests::tab_menus_copy_save_export_and_close` |
| DT-08 | Opening a `.proxymate`, HAR, `.chlsj` or `.chlsx` file adds a tab and makes it active | `lib/actions.ts` `openDocument` | G2 | ☑ | G2: `Workspace::open_document` (sessions, HAR, Charles); live: opened a `.proxymate` and a `.har` (PR) |
| DT-09 | Closing the active document returns to Live | `lib/actions.ts` `closeDocument` | G2 | ☑ | G2: `workspace::tests::closing_the_shown_document_returns_to_live_and_names_follow_the_tab` |
| DT-10 | Save Session suggests "<document>.proxymate" or "Proxymate YYYY-MM-DD HH.MM.proxymate" and saves in the background | `lib/actions.ts` `saveSession` | G2 | ☑ | G2: `Workspace::suggested_name`; `workspace::tests::closing_the_shown_document_returns_to_live_and_names_follow_the_tab` |
| DT-11 | Export HAR exports what the table shows by default, with the same naming | `lib/actions.ts` `exportHar` | G2 | ☑ | G2: `features/documents.rs` `save` (Har: the table's view by default) |

## Command palette & shortcuts

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| CP-01 | ⌘K toggles the palette: "Type a command…", groups Flows, Capture, Navigate, Windows, Settings, keyword matching, shortcut labels, only commands that apply now; "No matching command" | `components/CommandPalette.tsx` | G2 | ☑ | G2: `features/commands.rs` palette (groups, shortcuts, only registered commands that apply, "No matching command"); `tests/shell.rs` `the_palette_opens_with_cmd_k_and_runs_a_command`; `tests/render.rs` renders it in both themes |
| CP-02 | Escape in the inspector returns focus to the flow table (unless a dialog is open) | `hooks/useShortcuts.ts` | G3 | ☐ | |
| CP-03 | Shortcut rules: ⇧ never matches; ⌥ matches the physical key; ⌘⌫ and ⌘F are left to text fields and code editors; bare ⌫ and ⌘A only in the flow table | `lib/commands.ts` `commandForKey` | G2 | ☑ | G2: `features/commands.rs` `bind_keys` (exact bindings, so ⇧ never matches; gpui matches the physical key; ⌘⌫ yields to text fields via `!Input`; bare ⌫ and ⌘A are `FlowGrid`'s); `tests/shell.rs` `cmd_backspace_in_a_text_field_edits_the_text`. ⌘F in code editors is G4's |
| CP-04 | "Replay selected flow" ⌘R ("Nothing to replay", "Can't replay this flow" for tunnels and WebSockets) | `lib/commands.ts` | G8 | ☐ | |
| CP-05 | "Edit & Replay selected flow…" | `lib/commands.ts` | G8 | ☐ | |
| CP-06 | "Rewrite selected flow…" | `lib/commands.ts` | G9 | ☐ | |
| CP-07 | "Pin or unpin selected flow" (live capture only) | `lib/commands.ts` | G3 | ☐ | |
| CP-08 | "Copy URL of selected flow" (every selected URL with several) | `lib/commands.ts` | G3 | ☐ | |
| CP-09 | "Copy selected flow as <language>", one per Copy as Code language | `lib/commands.ts` | G11 | ☐ | |
| CP-10 | "Repeat selected flow…" ⌥⌘R | `lib/commands.ts` | G11 | ☐ | |
| CP-11 | "Select all flows" ⌘A (in the flow table) | `lib/commands.ts` | G1 | ☑ | G1: `pm-widgets` `flow_grid::tests::cmd_a_enter_and_backspace` (`flow_grid::SelectAll`, ⌘A in the `FlowGrid` context) |
| CP-12 | "Delete selected flows" ⌫ (in the flow table) | `lib/commands.ts` | G3 | ☐ | |
| CP-13 | "Replay selected flows in order" (two or more) | `lib/commands.ts` | G8 | ☐ | |
| CP-14 | "Compare the two selected flows" | `lib/commands.ts` | G11 | ☐ | |
| CP-15 | "Comment on selected flow…" | `lib/commands.ts` | G11 | ☐ | |
| CP-16 | "Label selected flows <Colour>", one per colour | `lib/commands.ts` | G11 | ☐ | |
| CP-17 | "Remove label from selected flows" | `lib/commands.ts` | G11 | ☐ | |
| CP-18 | "Add header column…" | `lib/commands.ts` | G11 | ☐ | |
| CP-19 | "Copy selected flow as cURL" | `lib/commands.ts` | G3 | ☐ | |
| CP-20 | "New request…" | `lib/commands.ts` | G8 | ☐ | |
| CP-21 | "Clear all flows" ⌘⌫ (live capture only) | `lib/commands.ts` | G2 | ☑ | G2: `features/capture.rs` (live only); `tests/shell.rs` `cmd_backspace_in_a_text_field_edits_the_text` |
| CP-22 | "Start recording" / "Stop recording" ⌘E (one key, whichever applies) | `lib/commands.ts` | G2 | ☑ | G2: `features/capture.rs`; `commands::tests::only_registered_commands_that_apply_are_listed_and_keys_pick_the_one_that_applies`, `tests/shell.rs` `the_palette_opens_with_cmd_k_and_runs_a_command` |
| CP-23 | "Toggle Auto Select" | `lib/commands.ts` | G1 | ☑ | G2: `features/capture.rs` (`auto-select`) in the palette |
| CP-24 | "Intercept: New Terminal window", "New iTerm window", "Launch Google Chrome", "Launch Microsoft Edge", "Launch Brave Browser", "Launch Arc", "Launch Firefox" (hidden when not installed) | `lib/commands.ts`, `lib/capture.ts` | G13 | ☐ | |
| CP-25 | "Toggle No Caching" | `lib/commands.ts` | G7 | ☐ | |
| CP-26 | "Toggle Block Cookies" | `lib/commands.ts` | G7 | ☐ | |
| CP-27 | "Auto-save now" | `lib/commands.ts` | G12 | ☐ | |
| CP-28 | "Open session, HAR or Charles file…" | `lib/commands.ts` | G2 | ☑ | G2: `features/documents.rs` (`open`) |
| CP-29 | "Mock from this session…" | `lib/commands.ts` | G12 | ☐ | |
| CP-30 | "Mock from a file…" | `lib/commands.ts` | G12 | ☐ | |
| CP-31 | "Import OpenAPI spec…", then Settings › Mocking | `lib/commands.ts` | G12 | ☐ | |
| CP-32 | "Save session…" | `lib/commands.ts` | G2 | ☑ | G2: `features/documents.rs` (`save-session`) |
| CP-33 | "Export HAR…" | `lib/commands.ts` | G2 | ☑ | G2: `features/documents.rs` (`export-har`) |
| CP-34 | "Search flows" ⌘F focuses and selects the search box | `lib/commands.ts` | G2 | ☑ | G2: `features/navigate.rs` (`search`, focuses and selects); `tests/shell.rs` `the_search_box_autocompletes_and_hands_back_to_the_table` |
| CP-35 | "Filter apps and domains" ⌘L | `lib/commands.ts` | G2 | ☑ | G2: `features/navigate.rs` (`filter`); `tests/shell.rs` `cmd_l_filters_the_sidebar` |
| CP-36 | "Show pinned flows" (`is:pinned`) | `lib/commands.ts` | G2 | ☑ | G2: `features/navigate.rs`; `tests/shell.rs` `the_palette_opens_with_cmd_k_and_runs_a_command` runs it from the palette |
| CP-37 | "Show labelled flows" (`is:labelled`) | `lib/commands.ts` | G2 | ☑ | G2: `features/navigate.rs` (`labelled`) |
| CP-38 | "Show commented flows" (`is:commented`) | `lib/commands.ts` | G2 | ☑ | G2: `features/navigate.rs` (`commented`) |
| CP-39 | "Search request and response bodies…" appends `body:` and focuses the box | `lib/commands.ts` | G2 | ☑ | G2: `features/navigate.rs` (`deep-search`, `pm_view::commands::with_body_term`, caret at the end) |
| CP-40 | "Focus the flow table" | `lib/commands.ts`, `lib/focus.ts` | G3 | ☐ | |
| CP-41 | "Focus the inspector" (with a selection) | `lib/commands.ts`, `lib/focus.ts` | G3 | ☐ | |
| CP-42 | "Show live capture" (on a document tab) | `lib/commands.ts` | G2 | ☑ | G2: `features/documents.rs` (`live`, only on a document tab); `tests/shell.rs` checks it is hidden on Live |
| CP-43 | "Rules…" | `lib/commands.ts` | G7 | ☐ | |
| CP-44 | "Scripts…" | `lib/commands.ts` | G10 | ☐ | |
| CP-45 | "Breakpoints…" | `lib/commands.ts` | G8 | ☐ | |
| CP-46 | "Device setup…" | `lib/commands.ts` | G13 | ☐ | |
| CP-47 | "Certificate wizard…" | `lib/commands.ts` | G13 | ☐ | |
| CP-48 | "Settings…" ⌘, | `lib/commands.ts` | G12 | ☐ | |
| CP-49 | "Settings: Proxy", "Network", "SSL Proxying", "Clients", "Storage", "Mocking", "Appearance", "Advanced", "MCP" | `lib/commands.ts` | G12 | ☐ | |
| CP-50 | "Appearance: System", "Appearance: Light", "Appearance: Dark" | `lib/commands.ts` | G2 | ☑ | G2: `features/navigate.rs` (`appearance-*`) |

## Inspector

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| IN-01 | Nothing selected: "Select a flow to inspect it" | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-02 | Load error: "Couldn't load this flow", the error, Try again | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-03 | Gone: "This flow is no longer available" | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-04 | While loading, a blank pane (no message flashing on every arrow key) | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-05 | URL bar: method pill, URL coloured for the bar (blue host, green path) with sent and GraphQL badges, status pill, HTTP version, "duration · size" | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-06 | URL bar: "Rewrite…" button (not CONNECT or WebSocket) | `components/DetailPane.tsx` | G9 | ☐ | |
| IN-07 | URL bar: "Bypass SSL" button on decrypted or pinned flows; "In SSL bypass" (disabled) once added; tooltip explains | `components/DetailPane.tsx` `BypassButton` | G3 | ☐ | |
| IN-08 | URL bar menu: Copy URL, Copy host, Copy path, Copy path and query, Copy method, Copy status, Copy as cURL | `components/DetailPane.tsx` `UrlBarMenu` | G3 | ☐ | |
| IN-09 | URL bar menu: Filter by ▸ Host, Method, Status, App | `components/DetailPane.tsx` `UrlBarMenu` | G3 | ☐ | |
| IN-10 | Notes bar: label chip and comment; click opens the comment dialog | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-11 | OpenAPI line: "OpenAPI" tag, operationId, summary, "path · spec" | `components/DetailPane.tsx` `OperationLine` | G3 | ☐ | |
| IN-12 | Rule badges: one per applied rule with req/resp, agent rules as "AI · <client>"; click opens the rule | `components/Inspector/RuleBadges.tsx` | G3 | ☐ | |
| IN-13 | Rule badge menu: Copy rule name, Copy rule id, Open rule, Disable rule | `components/Inspector/RuleBadges.tsx` | G3 | ☐ | |
| IN-14 | Rule notes: "Mocked: the response didn't come from the upstream.", "A breakpoint timed out; the flow continued unmodified.", why a rewrite skipped a body | `components/Inspector/RuleBadges.tsx` | G3 | ☐ | |
| IN-15 | Script failure badge "Script “<name>” failed · phase · message"; menu Copy error, Copy script name, Open script | `components/Inspector/RuleBadges.tsx` | G10 | ☐ | |
| IN-16 | Pinned banner (client rejected the certificate) with "Certificate wizard…" and "Add <host> to SSL bypass" | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-17 | Request and Response side by side (resizable, each at least 240 px); WebSocket and SSE flows add a stream panel below (40/60) | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-18 | Synthetic flow: "Synthetic flow: no recorded messages" | `components/DetailPane.tsx` `NoDetail` | G3 | ☐ | |
| IN-19 | Request tabs Header, Query, Body, Raw; the chosen tab stays across flows | `components/DetailPane.tsx` `RequestPane` | G3 | ☐ | |
| IN-20 | Response tabs Header, Body, Raw, TLS, Timing; the chosen tab stays across flows | `components/DetailPane.tsx` `ResponsePane` | G3 | ☐ | |
| IN-21 | Request Header tab: name/value table with display-cased names | `components/DetailPane.tsx`, `components/Inspector/KvTable.tsx` | G3 | ☐ | |
| IN-22 | Query tab: decoded pairs, or "No query parameters" | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-23 | Response Header tab, with a "Trailers" table when there are trailers | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-24 | No response: "Waiting for the response…" or "No response" | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-25 | Raw tab: "Rebuilt from the parsed message, not the exact bytes on the wire"; body placeholders for encoded, streaming, over 1 MB and binary bodies | `components/Inspector/RawView.tsx` | G3 | ☐ | |
| IN-26 | Raw menu: Copy request line / Copy status line, Copy headers, plus the code menu | `components/Inspector/RawView.tsx` | G3 | ☐ | |
| IN-27 | Tunnel note: "Not decrypted: <host> is in the SSL exclude or bypass list. N sent, N received through the tunnel." | `components/DetailPane.tsx` | G3 | ☐ | |
| IN-28 | TLS tab, Connection: Client SNI, Client offered ALPN, Negotiated ALPN, Upstream TLS version, Upstream cipher, Upstream certificate (Verified, Not verified, Failed), Upstream proxy, Client certificate | `components/Inspector/TlsView.tsx` | G3 | ☐ | |
| IN-29 | TLS tab, Upstream certificate chain: per certificate subject, leaf tag, Issuer, Valid (with "expired when captured"), Names, Serial, SHA-256 | `components/Inspector/TlsView.tsx` `CertCard` | G3 | ☐ | |
| IN-30 | TLS placeholders: not decrypted, "Plain HTTP: no TLS", "The upstream presented no certificate" | `components/Inspector/TlsView.tsx` | G3 | ☐ | |
| IN-31 | Certificate menu: Copy subject, Copy SHA-256, Copy SHA-256 without colons, Copy serial, Copy names | `components/Inspector/TlsView.tsx` `CertCard` | G3 | ☐ | |
| IN-32 | Timing tab: waterfall of phases with times, Total, "Reused an open upstream connection", route rows | `components/Inspector/TimingView.tsx` | G3 | ☐ | |
| IN-33 | Timing placeholders: "Waiting for the upstream…", "No timings recorded" | `components/Inspector/TimingView.tsx` | G3 | ☐ | |
| IN-34 | Timing menu: Copy <phase>, Copy timing as table (tab-separated) | `components/Inspector/TimingView.tsx` `TimingMenu` | G3 | ☐ | |
| IN-35 | Table row menu: Copy name, Copy value, Copy as "Name: value" (headers) / Copy as name=value (query, form) / Copy row (plain) | `components/Inspector/KvTable.tsx` `RowItems` | G3 | ☐ | |
| IN-36 | Table row menu: Copy token (Authorization), Copy cookie value / Copy cookie ▸ <name> | `components/Inspector/KvTable.tsx` `RowItems` | G3 | ☐ | |
| IN-37 | Table row menu: Filter flows with this header | `components/Inspector/KvTable.tsx` `RowItems` | G3 | ☐ | |
| IN-38 | Table menu: Copy all headers / parameters / fields / rows, Copy all as JSON | `components/Inspector/KvTable.tsx` `KvMenu` | G3 | ☐ | |
| IN-39 | Failure banner: headline and the engine's message; menu Copy error, Copy title and error | `components/DetailPane.tsx` `FailureBanner` | G3 | ☐ | |
| IN-40 | Failure headlines by error kind (client rejected cert, client TLS, client cert required, upstream proxy auth, loop, unreachable, cert untrusted, cert invalid, old TLS, upstream TLS, DNS, timeout, refused, client gone, other) | `lib/errors.ts` | G3 | ☐ | |
| IN-41 | Fix buttons: Certificate wizard…, Bypass SSL, Allow insecure upstream / Verify upstream again, Timeouts…, Network settings…, SSL Proxying… | `components/DetailPane.tsx` `FailureBanner` | G3 | ☐ | |
| IN-42 | Text in the URL bar, tables and notes is selectable | `components/DetailPane.tsx` | G3 | ☐ | |

## Body viewers

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| BV-01 | Viewer picker: the viewers for the body's kind, best first; the choice sticks per content kind | `components/BodyViewer/BodyViewer.tsx`, `components/BodyViewer/registry.ts` | G4 | ☐ | |
| BV-02 | Kind detection, falling back to the Content-Type for flows recorded before detection or still streaming | `components/BodyViewer/BodyViewer.tsx` | G4 | ☐ | |
| BV-03 | Body bar: "mime · size", Copy (as text), Save… | `components/BodyViewer/BodyViewer.tsx` | G4 | ☐ | |
| BV-04 | "No body", "Waiting for body…", "Body streaming · N so far" | `components/BodyViewer/BodyViewer.tsx` | G4 | ☐ | |
| BV-05 | "Decoded from <encoding> · a → b"; "Couldn't decode: … Showing the raw bytes." | `components/BodyViewer/BodyViewer.tsx` | G4 | ☐ | |
| BV-06 | "Recorded the first X of Y" for truncated bodies | `components/BodyViewer/BodyViewer.tsx` | G4 | ☐ | |
| BV-07 | Bodies over 20 MB: "Showing the first 20 MB of …" with Load all and Save body to file… | `components/BodyViewer/BodyViewer.tsx`, `components/BodyViewer/util.ts` | G4 | ☐ | |
| BV-08 | Highlighting off over 2 MB, with Highlight anyway | `components/BodyViewer/BodyViewer.tsx` | G4 | ☐ | |
| BV-09 | "Loading…", "Couldn't load body: …", "Couldn't save: …" | `components/BodyViewer/BodyViewer.tsx` | G4 | ☐ | |
| BV-10 | Body menu (viewers without their own): Copy (selection), Copy body, Copy body as Base64, Save body… | `components/menus/body.tsx` | G4 | ☐ | |
| BV-11 | Code view: read-only, line numbers, folding, bracket matching, ⌘F search, smooth on multi-megabyte bodies | `components/BodyViewer/CodeView.tsx`, `components/BodyViewer/code/` | G4 | ☐ | |
| BV-12 | Code menu: Copy (selection), Copy all, Select all ⌘A, Find… ⌘F, then the body items | `components/BodyViewer/CodeView.tsx` | G4 | ☐ | |
| BV-13 | Body search from the filter's body terms: matches marked, "N matches" / "i of N" bar with previous and next | `components/BodyViewer/search.tsx`, `components/BodyViewer/searchTerms.ts` | G4 | ☐ | |
| BV-14 | JSON: pretty-printed and highlighted (formatted off the main thread above 256 KB) | `components/BodyViewer/JsonViews.tsx` `JsonPretty` | G4 | ☐ | |
| BV-15 | Invalid JSON: "Not valid JSON: … Showing the text." | `components/BodyViewer/JsonViews.tsx` `InvalidJson` | G4 | ☐ | |
| BV-16 | JSON Tree: virtualised, open to depth 1, click toggles, ⌥-click expands the subtree; numbers exact | `components/BodyViewer/JsonViews.tsx` `Tree` | G4 | ☐ | |
| BV-17 | JSON Tree filter "$.items[*].id or /items/0" with match count or error | `components/BodyViewer/JsonViews.tsx` `Tree` | G4 | ☐ | |
| BV-18 | JSON Tree menu: Copy value, Copy key, Copy path, Copy JSON pointer, Expand all below, Collapse all | `components/BodyViewer/JsonViews.tsx` `Tree` | G4 | ☐ | |
| BV-19 | GraphQL: Query, Variables and Extensions panes; operation picker for batches; GET operations from the query string; "Not a GraphQL request body. Showing the text." | `components/BodyViewer/GraphQlView.tsx` | G4 | ☐ | |
| BV-20 | GraphQL menu: Copy operation name, Copy query, Copy variables | `components/BodyViewer/GraphQlView.tsx` | G4 | ☐ | |
| BV-21 | XML and HTML: re-indented (up to 2 MB) and highlighted | `components/BodyViewer/TextViews.tsx` `MarkupView` | G4 | ☐ | |
| BV-22 | JavaScript and CSS: Beautify toggle (on by default for minified code), "Formatting…", "Couldn't beautify: …" | `components/BodyViewer/TextViews.tsx` `ScriptView` | G4 | ☐ | |
| BV-23 | Text: Wrap toggle; "Binary body: switch to Hex to see its bytes." | `components/BodyViewer/TextViews.tsx` `TextView` | G4 | ☐ | |
| BV-24 | Form: Fields / Raw toggle; Name and Value table with the table menu | `components/BodyViewer/FormView.tsx` | G4 | ☐ | |
| BV-25 | Multipart: a card per part with its headers and a fitting sub-viewer, images inline; "The Content-Type has no boundary…", "No parts found for boundary …" | `components/BodyViewer/MultipartView.tsx` | G4 | ☐ | |
| BV-26 | Multipart part menu: Copy name, Copy file name, Copy content type | `components/BodyViewer/MultipartView.tsx` | G4 | ☐ | |
| BV-27 | HTTP syntax highlighting for Raw messages | `components/BodyViewer/code/languages.ts` | G4 | ☐ | |
| BV-28 | Hex: Offset, hex and ASCII, 16 bytes a row, virtualised | `components/BodyViewer/HexView.tsx` | G4 | ☐ | |
| BV-29 | Hex menu: Copy byte 0x…, Copy row as hex, Copy row as text, Copy offset, Copy as hex dump | `components/BodyViewer/HexView.tsx` | G4 | ☐ | |
| BV-30 | Image: checkerboard, Fit, Actual size (100%), Zoom out, Zoom in; dimensions, format (animated), size, EXIF orientation | `components/BodyViewer/MediaViews.tsx` `ImageView` | G5 | ☐ | |
| BV-31 | Image: Save image…, Copy image buttons; menu Copy image, Copy dimensions, Save image…; "can't decode this image" | `components/BodyViewer/MediaViews.tsx` `ImageView` | G5 | ☐ | |
| BV-32 | SVG rendered as an image: "its scripts and external resources never load" | `components/BodyViewer/MediaViews.tsx` `ImageView` | G5 | ☐ | |
| BV-33 | Video and audio players with format, duration, dimensions, size; error message | `components/BodyViewer/MediaViews.tsx` `MediaView` | G5 | ☐ | |
| BV-34 | PDF viewer with "PDF · size" | `components/BodyViewer/MediaViews.tsx` `PdfView` | G5 | ☐ | |
| BV-35 | Font: editable sample text in the font; "Loading font…", "Couldn't load this font: …" | `components/BodyViewer/MediaViews.tsx` `FontView` | G5 | ☐ | |
| BV-36 | HTML Preview: rendered with scripts, forms and network loads blocked; note says so | `components/BodyViewer/TextViews.tsx` `HtmlPreview`, `lib/body/htmlPreview.ts` | G5 | ☐ | |
| BV-37 | gRPC: message list (index, size, compressed), the selected message as a tree, status and trailers; a live stream fills in ("Streaming · …") | `components/BodyViewer/DecodedView.tsx` | G5 | ☐ | |
| BV-38 | Decoded (protobuf, MessagePack, CBOR): tree with badges; "Decoding…", "Couldn't decode: …", partial-message warnings, "No messages" | `components/BodyViewer/DecodedView.tsx` | G5 | ☐ | |
| BV-39 | "Decode as: automatic" or a message type, saved as a mapping | `components/BodyViewer/DecodedView.tsx` `DecodeAs` | G5 | ☐ | |
| BV-40 | View scripts in the viewer picker under "Scripts" (first view shown unless a built-in was picked); "<script> · mime" in the bar | `components/BodyViewer/BodyViewer.tsx`, `components/BodyViewer/ScriptViewBody.tsx` | G10 | ☐ | |
| BV-41 | "View script “<name>” failed: …" with Open script; "The script's view was cut at 8 MB." | `components/BodyViewer/BodyViewer.tsx` | G10 | ☐ | |

## Streams

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| ST-01 | WebSocket panel header: "WebSocket", live dot with Open / close description / Closed, "N messages" or "a of b messages" | `components/Inspector/WebSocketPanel.tsx` | G6 | ☐ | |
| ST-02 | Direction filter All, Sent, Received | `components/Inspector/WebSocketPanel.tsx` | G6 | ☐ | |
| ST-03 | "Search payloads" box with clear | `components/Inspector/StreamShell.tsx` `SearchBox` | G6 | ☐ | |
| ST-04 | Close button on a live open connection ("a normal close frame to each side") | `components/Inspector/WebSocketPanel.tsx` | G6 | ☐ | |
| ST-05 | Message list: direction arrow, #, Time, Type, Size, Payload preview; follows the end until scrolled up; "No messages yet" / "No messages match" | `components/Inspector/WebSocketPanel.tsx`, `components/Inspector/StreamShell.tsx` `MessageList` | G6 | ☐ | |
| ST-06 | Message flags Injected, Edited, Dropped with tooltips; dropped rows struck through | `components/Inspector/WebSocketPanel.tsx` `Flags` | G6 | ☐ | |
| ST-07 | Click selects a message; with Auto Select the newest is inspected until one is clicked | `components/Inspector/WebSocketPanel.tsx` | G6 | ☐ | |
| ST-08 | Message detail lines: who sent it, type, bytes, time; close code; truncation; dropped; Resend button | `components/Inspector/WebSocketPanel.tsx` `MessageDetail` | G6 | ☐ | |
| ST-09 | Edited message: Forwarded / Original toggle | `components/Inspector/WebSocketPanel.tsx` `MessageDetail` | G6 | ☐ | |
| ST-10 | Payload view: viewer picker (bytes viewers; hex first for binary), "mime · size", "Empty payload", body menu | `components/Inspector/PayloadView.tsx` | G6 | ☐ | |
| ST-11 | Composer: To server / To client, Text / Binary (hex), message box, Send (⌘↩), inline error | `components/Inspector/WebSocketPanel.tsx` `Composer` | G6 | ☐ | |
| ST-12 | Message menu: Copy payload (or Copy close code and reason), Copy payload as Base64, Copy payload as hex, Resend to the server / to the client | `components/Inspector/WebSocketPanel.tsx` `MessageItems` | G6 | ☐ | |
| ST-13 | "Select a message", "Loading…", "Couldn't load the payload: …" | `components/Inspector/WebSocketPanel.tsx` | G6 | ☐ | |
| ST-14 | SSE panel header: "Events", Streaming / Stream ended, event count, "Search events" | `components/Inspector/SsePanel.tsx` | G6 | ☐ | |
| ST-15 | SSE list: Time, Event, Id, Data; "Waiting for events…", "No events", "No events match" | `components/Inspector/SsePanel.tsx` | G6 | ☐ | |
| ST-16 | SSE event detail: id, retry, time; "Data cut at 1 MB"; data in the payload view; "Select an event" | `components/Inspector/SsePanel.tsx` | G6 | ☐ | |
| ST-17 | SSE menu: Copy data, Copy as event-stream text, Copy event name, Copy id | `components/Inspector/SsePanel.tsx` | G6 | ☐ | |
| ST-18 | List and detail side by side, resizable | `components/Inspector/StreamShell.tsx` | G6 | ☐ | |

## Rules

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| RU-01 | Rules window: title, description, rule list beside the form; "Select a rule, or create one from a flow's context menu." | `components/Rules/RulesEditor.tsx` | G7 | ☐ | |
| RU-02 | New rule button | `components/Rules/RuleList.tsx` | G7 | ☐ | |
| RU-03 | New group button | `components/Rules/RuleList.tsx` | G7 | ☐ | |
| RU-04 | Remove agent rules (when an agent made any) | `components/Rules/RuleList.tsx` | G7 | ☐ | |
| RU-05 | Unsaved draft shown as "Unsaved rule"; "No rules yet." | `components/Rules/RuleList.tsx` | G7 | ☐ | |
| RU-06 | Rule row: grip, enabled checkbox (tooltips for on, off, group off), name or "Untitled", origin badge, matcher summary, action summary; muted when off | `components/Rules/RuleList.tsx` `RuleRow`, `components/Rules/RuleOriginBadge.tsx` | G7 | ☐ | |
| RU-07 | Drag rules to reorder, into a group or back to the ungrouped section | `components/Rules/RuleList.tsx` | G7 | ☐ | |
| RU-08 | Group header: collapse chevron, on/off checkbox, name, "n of m rules enabled"; double-click renames inline (↩ saves, Escape cancels) | `components/Rules/RuleList.tsx` `GroupHeader` | G7 | ☐ | |
| RU-09 | Empty group: "No rules: drag rules here, or use a rule's Move to Group menu." | `components/Rules/RuleList.tsx` | G7 | ☐ | |
| RU-10 | Rule menu: Open, Enable / Disable, Duplicate | `components/Rules/RuleList.tsx` `RuleItems` | G7 | ☐ | |
| RU-11 | Rule menu: Edit visually… (visual rewrites) | `components/Rules/RuleList.tsx` `RuleItems` | G9 | ☐ | |
| RU-12 | Rule menu: Move to top, Move to bottom (within its section) | `components/Rules/RuleList.tsx` `RuleItems` | G7 | ☐ | |
| RU-13 | Rule menu: Move to group ▸ each group, New group…, No group | `components/Rules/RuleList.tsx` `RuleItems` | G7 | ☐ | |
| RU-14 | Rule menu: Copy name, Copy as JSON, Delete | `components/Rules/RuleList.tsx` `RuleItems` | G7 | ☐ | |
| RU-15 | Group menu: Turn group on / off, New rule in group, Rename… | `components/Rules/RuleList.tsx` `GroupItems` | G7 | ☐ | |
| RU-16 | Group menu: Move up, Move down, Copy name | `components/Rules/RuleList.tsx` `GroupItems` | G7 | ☐ | |
| RU-17 | Group menu: Delete group, keep its rules; Delete group and its N rules | `components/Rules/RuleList.tsx` `GroupItems` | G7 | ☐ | |
| RU-18 | Breakpoint timeout field under the list (seconds, 0 waits forever) | `components/Rules/RuleList.tsx` `BreakpointTimeout` | G7 | ☐ | |
| RU-19 | Rule form header: name, "Disabled", origin badge, group picker ("No group"), "Group off", "Unsaved" | `components/Rules/RulesEditor.tsx` `RuleForm` | G7 | ☐ | |
| RU-20 | Rule form: Edit visually button for rules with visual rewrites | `components/Rules/RulesEditor.tsx` `RuleForm` | G9 | ☐ | |
| RU-21 | Live validation as you type (engine compiles the rule), error in the footer; Delete; Save / Save rule (disabled when invalid or unchanged) | `components/Rules/RulesEditor.tsx` `RuleForm` | G7 | ☐ | |
| RU-22 | Match: Methods (GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS) | `components/Rules/MatcherBuilder.tsx` | G7 | ☐ | |
| RU-23 | Match: Scheme (Any, http, https) · port | `components/Rules/MatcherBuilder.tsx` | G7 | ☐ | |
| RU-24 | Match: Host and Path patterns with an Exact / Glob / Regex picker | `components/Rules/MatcherBuilder.tsx`, `components/Rules/fields.tsx` `PatternInput` | G7 | ☐ | |
| RU-25 | Match: Query and Headers rows (name, value pattern or "present", not, remove; + Query parameter, + Header) | `components/Rules/MatcherBuilder.tsx` `KvRows` | G7 | ☐ | |
| RU-26 | Match: Client app pattern | `components/Rules/MatcherBuilder.tsx` | G7 | ☐ | |
| RU-27 | Match: "Only for some responses" adds Response status ranges and Content-Type; removable | `components/Rules/MatcherBuilder.tsx` `ResponseMatcherRow` | G7 | ☐ | |
| RU-28 | Actions: Add action menu grouped Headers, Query, Body, Status, Routing, Flow control, Scripts, WebSocket, Annotate; "No actions yet: the rule matches but does nothing." | `components/Rules/ActionListEditor.tsx`, `lib/rules.ts` `ACTION_TYPES` | G7 | ☐ | |
| RU-29 | Action row: phase picker Request / Response, Move up, Move down, Remove action | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-30 | Actions Set header, Add header value, Remove header, Replace in header | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-31 | Actions Set query parameter, Remove query parameter | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-32 | Actions Set body (with Content-Type), Replace in body (Find, Replace with, "$1 for a regex group") | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-33 | Actions JSON set (JSON pointer, Value (JSON)), JSON remove | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-34 | Actions Form field set, Form field remove | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-35 | Action Set status | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-36 | Action Map Local: Answer from A file / A directory / Inline body, path with Choose… | `components/Rules/ActionListEditor.tsx`, `components/PathInput.tsx` | G7 | ☐ | |
| RU-37 | Action Map Remote: Scheme, Host · port, Path, Keep the original Host header | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-38 | Action Serve from recording: Recording picker, Also compare (query, request body), Several match, None match | `components/Rules/ActionListEditor.tsx` `ServeFromForm` | G7 | ☐ | |
| RU-39 | Action Breakpoint (Pause at request or response) | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-40 | Action Delay (ms) | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-41 | Action Throttle: Down · up (kbps, 0 unlimited), Latency | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-42 | Action Block with A status or A connection reset | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-43 | Action Run script: script picker ("Choose a script…", "(missing script)") | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-44 | Action WebSocket message: Messages (Both ways, Sent by the client, From the server), Containing, Do Replace / JSON set / JSON remove / Drop / Delay / Breakpoint | `components/Rules/ActionListEditor.tsx`, `lib/rules.ts` `WS_OP_LABELS` | G7 | ☐ | |
| RU-45 | Action Comment on flow | `components/Rules/ActionListEditor.tsx` | G7 | ☐ | |
| RU-46 | Dry run: Test against selected flow ("Select a flow in the table first"); match or why not; resulting request and response; menu Copy headers, Copy body, Copy notes | `components/Rules/RuleTestPanel.tsx` | G7 | ☐ | |
| RU-47 | Presets (Map Local…, Breakpoint…) pre-fill a rule from the flow and open it unsaved | `lib/actions.ts` `openPreset` | G7 | ☐ | |
| RU-48 | Opening a rule by id (badges, breakpoints, scripts) selects it in the window | `store/ui.ts` `openRulesEditor` | G7 | ☐ | |
| RU-49 | Built-in rules (throttle, No Caching, Block Cookies) are hidden from the list | `components/Rules/RulesEditor.tsx`, `lib/rules.ts` | G7 | ☐ | |
| RU-50 | Throttle popover "Throttle all traffic": Off (Full speed), LTE, Fast 3G, 3G, Edge, Very bad network with speeds and latency; ✓ on the active one; Custom when edited elsewhere | `components/Rules/ThrottleMenu.tsx`, `lib/rules.ts` `THROTTLE_PRESETS` | G7 | ☐ | |

## Breakpoints & editing

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| BE-01 | Breakpoint Editor window: the queue beside the selected item; "Nothing is paused." | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-02 | Queue item: phase pill, rule name, countdown m:ss or "no timeout" | `components/Breakpoints/BreakpointEditor.tsx` `QueueItem` | G8 | ☐ | |
| BE-03 | Queue item menu: Copy URL, Copy method, Show flow #N, Open rule "<name>", Continue unmodified, Abort / Drop the message | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-04 | Paused request: method, URL, headers (add, remove), body | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-05 | Paused response: Status, headers, body (size and note) | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-06 | Execute (with edits) and Abort | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-07 | Respond… shows "Answer the client with" status, headers, body, then Send response | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-08 | Held WebSocket message: Text / Tree / Hex, hex and UTF-8 validation messages, Send and Drop | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-09 | Body editor: Text / Tree for JSON; "The text isn't valid JSON" | `components/Breakpoints/BreakpointEditor.tsx` | G8 | ☐ | |
| BE-10 | JSON editor: virtualised editable tree, typed inline editors, rows tinted where they differ from the original, count of removed members | `components/JsonEditor/JsonEditor.tsx` | G8 | ☐ | |
| BE-11 | JSON editor keys: ↑↓ move, ←→ collapse/expand, ↩ edit, F2 rename, ⌘↩ add after, ⌘D duplicate, ⌫ remove, ⌘Z / ⇧⌘Z / ⌘Y, Escape cancels | `components/JsonEditor/JsonEditor.tsx` | G8 | ☐ | |
| BE-12 | JSON editor toolbar: Undo, Redo, Expand all, Collapse all | `components/JsonEditor/JsonEditor.tsx` | G8 | ☐ | |
| BE-13 | JSON editor menu: edit value, Rename, change type ▸, Duplicate, Reset, Remove, Copy value, Copy JSON pointer; add-member menu by type | `components/JsonEditor/JsonEditor.tsx` | G8 | ☐ | |
| BE-14 | JSON editor rejects a key already used in the object | `components/JsonEditor/JsonEditor.tsx` | G8 | ☐ | |
| BE-15 | Name/value editor: changed rows tinted, removed rows struck through with Put back, repeated names removable only, framing headers locked; menu Copy name, Copy value | `components/Rewrite/KvEditor.tsx` | G8 | ☐ | |
| BE-16 | Compose dialog: method, URL, headers (add, remove), body with content type (None, JSON, Form, Text, XML, HTML, JavaScript, GraphQL, custom), Apply rules switch, Send (⌘↩), error line | `components/Compose/ComposeDialog.tsx`, `lib/compose.ts` | G8 | ☐ | |
| BE-17 | Edit & Replay: Compose titled "Edit & Replay", prefilled from the flow; a binary body is "sent as recorded" with Clear | `components/Compose/ComposeDialog.tsx` | G8 | ☐ | |
| BE-18 | Compose body editor: search, history, indent with Tab | `components/Compose/CodeMirrorEditor.tsx` | G8 | ☐ | |
| BE-19 | Sending records a new live flow marked Replay and selects it | `lib/actions.ts` `sendComposed` | G8 | ☐ | |
| BE-20 | Replay sends the recorded request again as a new flow | `lib/actions.ts` `replayFlow` | G8 | ☐ | |
| BE-21 | Replay in Order sends the selection one after another | `lib/actions.ts` `replaySelection` | G8 | ☐ | |

## Rewrite

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| RW-01 | Rewrite dialog titled "Rewrite" (or "Edit rewrite" for a saved rule); "Loading the flow…" | `components/Rewrite/RewriteDialog.tsx` | G9 | ☐ | |
| RW-02 | Opens from Rewrite… on a flow (live or document), the URL bar and Edit visually | `components/Rewrite/RewriteDialog.tsx`, `lib/actions.ts` `openRewrite` | G9 | ☐ | |
| RW-03 | Rule name field | `components/Rewrite/RewriteEditor.tsx` | G9 | ☐ | |
| RW-04 | Side switch Request / Response with change counts | `components/Rewrite/RewriteEditor.tsx` | G9 | ☐ | |
| RW-05 | Tabs: Request Headers, Query, Body; Response Status, Headers, Body | `components/Rewrite/RewriteEditor.tsx` | G9 | ☐ | |
| RW-06 | Headers and Query editing (framing headers "Set by the proxy when the body changes") | `components/Rewrite/RewriteEditor.tsx`, `components/Rewrite/KvEditor.tsx` | G9 | ☐ | |
| RW-07 | Status editing | `components/Rewrite/RewriteEditor.tsx` | G9 | ☐ | |
| RW-08 | Body: JSON tree, form fields, text with Replace whole body / Find & replace pairs; "No body." | `components/Rewrite/BodyPane.tsx` | G9 | ☐ | |
| RW-09 | "Applies to" chips: Exact URL, This path with these query parameters, This path, any query string, Path glob (id-like segments become *), Ignore query, Advanced… | `components/Rewrite/MatcherChips.tsx` | G9 | ☐ | |
| RW-10 | Preview: engine dry run "The <side> after the rule" or "No match: …", header diff, body line diff | `components/Rewrite/Preview.tsx` | G9 | ☐ | |
| RW-11 | Actions panel: generated actions, "Kept from the saved rule", problems; "Edit the flow to add actions." | `components/Rewrite/RewriteEditor.tsx`, `components/Rewrite/parts.tsx` | G9 | ☐ | |
| RW-12 | Footer: "No changes yet." / "N changes · kept · problems", Cancel, Create rule / Save rule | `components/Rewrite/RewriteEditor.tsx` | G9 | ☐ | |
| RW-13 | Edits become patches of only what changed (never a re-serialised body) | `lib/rewrite/diff.ts`, `lib/rewrite/session.ts` | G9 | ☐ | |

## Scripts

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SC-01 | Scripts window: list grouped View, Rewrite, Mock; "No scripts yet."; "Select a script, or start one from a template." | `components/Scripts/ScriptsWindow.tsx` | G10 | ☐ | |
| SC-02 | New script popover: templates by kind (Decode base64 JSON field, Decode JWT, Add a header, Modify a JSON field, Rewrite WebSocket messages, Mock JSON) with descriptions | `components/Scripts/ScriptsWindow.tsx` `NewScriptMenu`, `lib/scripts.ts` | G10 | ☐ | |
| SC-03 | Import… a .js file | `components/Scripts/ScriptsWindow.tsx` | G10 | ☐ | |
| SC-04 | Remove agent scripts (agent-made scripts no rule runs) | `components/Scripts/ScriptsWindow.tsx` | G10 | ☐ | |
| SC-05 | Script row: enabled state, name or "Untitled", origin badge | `components/Scripts/ScriptsWindow.tsx` | G10 | ☐ | |
| SC-06 | Script menu: Open, Enable / Disable, Duplicate, Create rule for selected flow…, Export…, Copy name, Copy source, Delete | `components/Scripts/ScriptsWindow.tsx` | G10 | ☐ | |
| SC-07 | Script header: name, kind picker for new scripts (else "<Kind> script"), Enabled, origin badge, "Unsaved"; help line for the kind | `components/Scripts/ScriptsWindow.tsx` | G10 | ☐ | |
| SC-08 | Code editor: JavaScript with completions for the script API | `components/Scripts/ScriptCodeMirror.tsx`, `lib/scripts.ts` | G10 | ☐ | |
| SC-09 | "Where it runs" tab (view scripts): Bodies (Response bodies, Request bodies), content kinds ("None ticked means all."), Flows matcher | `components/Scripts/ScriptsWindow.tsx` `ViewSettings` | G10 | ☐ | |
| SC-10 | "Used by rules" tab: rules that run it ("(off)"), empty messages, create a rule for the selected flow | `components/Scripts/ScriptsWindow.tsx` `UsedByRules` | G10 | ☐ | |
| SC-11 | Test tab: Response body / Request body, Test against selected flow; request, response or mocked response; console; menu Copy body, Copy error | `components/Scripts/ScriptTestPanel.tsx` | G10 | ☐ | |
| SC-12 | Console tab: entries with level colours, This script only, Clear console, flow link "Select this flow"; menu Copy message, Copy line, copy all, Show flow #N, Clear console; "Nothing logged yet." | `components/Scripts/ScriptConsole.tsx` | G10 | ☐ | |
| SC-13 | Footer: error, Copy type definitions, Export…, Delete, Save / Save script | `components/Scripts/ScriptsWindow.tsx` | G10 | ☐ | |
| SC-14 | Saving evaluates the script and shows its error | `lib/actions.ts` `saveScript` | G10 | ☐ | |

## Workflow

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| WF-01 | Comment dialog: "What's special about this flow?", description, Remove, Cancel, Save | `components/Workflow/SmallDialogs.tsx` `CommentDialog` | G11 | ☐ | |
| WF-02 | Add header column dialog: Request / Response, header name, "already there or 8 already" error, Cancel, Add | `components/Workflow/SmallDialogs.tsx` `AddColumnDialog` | G11 | ☐ | |
| WF-03 | Header columns apply to every store; values fill in for existing flows in the background | `components/FlowTable/FlowTable.tsx`, `store/prefs.ts` | G11 | ☐ | |
| WF-04 | Compare dialog: A and B, "Left is the older flow…"; tabs Request Headers (n), Response Headers (n), Request Body, Response Body, Timing | `components/Workflow/CompareDialog.tsx` | G11 | ☐ | |
| WF-05 | Compare: "Ignore volatile headers (date, request ids, traces)" switch | `components/Workflow/CompareDialog.tsx` | G11 | ☐ | |
| WF-06 | Compare: header diff by name, body line diff, timing table (Connect, Waiting (TTFB), Receiving, Total) with deltas | `components/Workflow/CompareDialog.tsx`, `lib/compare.ts` | G11 | ☐ | |
| WF-07 | Compare: "This flow is gone", "Loading…", "No headers to compare." | `components/Workflow/CompareDialog.tsx` | G11 | ☐ | |
| WF-08 | Repeat dialog: Iterations, Concurrency, Delay (ms), Apply rules; Start / Run Again, Stop, Close; results by status; "Done, N failed." / "Stopped."; Show Flows | `components/Workflow/RepeatDialog.tsx` | G11 | ☐ | |
| WF-09 | Labels (six colours) show in the table's Notes column and the inspector's notes bar | `components/FlowTable/FlowRow.tsx`, `components/DetailPane.tsx` | G11 | ☐ | |
| WF-10 | Copy as Code renders the request in each language | `lib/actions.ts` `copyFlowCode` | G11 | ☐ | |
| WF-11 | Bulk actions take the selection as ids or a whole view with exceptions | `lib/selection.ts`, `lib/actions.ts` `currentSelection` | G11 | ☐ | |

## Settings

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-01 | Settings window: page list (Proxy, Network, SSL Proxying, Clients, Storage, Protobuf, Mocking, Appearance, Advanced, MCP, Updates) beside the page; opens on a given page | `components/Settings/SettingsDialog.tsx` | G12 | ☐ | |

### Proxy

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-PX-01 | Listening: status line with menu Copy address, Copy error | `components/Settings/ProxyPage.tsx` | G12 | ☐ | |
| SE-PX-02 | Port (1024–65535) with Apply ("Restarting…") | `components/Settings/ProxyPage.tsx` | G12 | ☐ | |
| SE-PX-03 | LAN access switch; "Address shown to devices" interface picker with Copy IP address, Copy address and port | `components/Settings/ProxyPage.tsx` | G12 | ☐ | |
| SE-PX-04 | System proxy switch | `components/Settings/ProxyPage.tsx` | G12 | ☐ | |
| SE-PX-05 | Proxied terminal: environment to copy | `components/Settings/ProxyPage.tsx` | G12 | ☐ | |

### Network

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-NW-01 | Upstream proxies: rules with Enabled, Hosts, Through (Direct, HTTP, SOCKS5), Proxy host, Port, Username, Password; Add upstream rule; Remove | `components/Settings/NetworkPage.tsx` | G12 | ☐ | |
| SE-NW-02 | Upstream bypass list ("Nothing bypasses the upstream rules"); "Use the proxy that was set before Proxymate for hosts no rule matches" | `components/Settings/NetworkPage.tsx` | G12 | ☐ | |
| SE-NW-03 | SOCKS5: description, Copy SOCKS5 address, Copy curl example | `components/Settings/NetworkPage.tsx` | G12 | ☐ | |
| SE-NW-04 | Reverse proxies: Local port, Forwards to, TLS, Keep Host; Add reverse proxy | `components/Settings/NetworkPage.tsx` | G12 | ☐ | |
| SE-NW-05 | Client certificates: Host, Format (PEM, PKCS#12), certificate/key or .p12 file with Choose…, password; Add client certificate | `components/Settings/NetworkPage.tsx` | G12 | ☐ | |
| SE-NW-06 | DNS overrides: Host, Connect to; Add DNS override | `components/Settings/NetworkPage.tsx` | G12 | ☐ | |
| SE-NW-07 | Save ("Saving…"), Revert, errors | `components/Settings/NetworkPage.tsx` | G12 | ☐ | |

### SSL Proxying

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-SSL-01 | Decrypt list ("* decrypts everything not excluded below") | `components/Settings/SslPage.tsx` | G12 | ☐ | |
| SE-SSL-02 | Never decrypt list (Apple defaults) | `components/Settings/SslPage.tsx` | G12 | ☐ | |
| SE-SSL-03 | SSL bypass list | `components/Settings/SslPage.tsx` | G12 | ☐ | |
| SE-SSL-04 | Allow insecure upstream list | `components/Settings/SslPage.tsx` | G12 | ☐ | |
| SE-SSL-05 | Apps never decrypted list (bundle ids or tool names) | `components/Settings/SslPage.tsx` | G12 | ☐ | |
| SE-SSL-06 | List editor: Add, Remove <item>, empty text, Copy <item> menu | `components/Settings/parts.tsx` `ListEditor` | G12 | ☐ | |

### Clients

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-CL-01 | LAN clients: status, nickname, Allow / Save, Deny, Forget; "No devices have connected yet" | `components/Settings/ClientsPage.tsx` | G12 | ☐ | |
| SE-CL-02 | Device menu: Copy IP address, Copy nickname, Allow, Deny, Forget | `components/DeviceSetup/parts.tsx` `DeviceMenu` | G12 | ☐ | |

### Storage

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-STO-01 | Live capture: Location, Flows, On disk, Evicted since launch, Not recorded since launch; Clear all flows | `components/Settings/StoragePage.tsx` | G12 | ☐ | |
| SE-STO-02 | Never record list | `components/Settings/StoragePage.tsx` | G12 | ☐ | |
| SE-STO-03 | Only record list | `components/Settings/StoragePage.tsx` | G12 | ☐ | |
| SE-STO-04 | Auto-save: switch, Folder with Choose…, Every (minutes), Keep (sessions), Save, Save now | `components/Settings/StoragePage.tsx` | G12 | ☐ | |
| SE-STO-05 | Limits: Flows kept, Disk used (MB), WebSocket messages per connection; Save limits, Revert | `components/Settings/StoragePage.tsx` | G12 | ☐ | |

### Protobuf

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-PB-01 | Schemas: Import… (.proto or descriptor set), list with files, Remove <name>, how-to text | `components/Settings/ProtobufPage.tsx` | G12 | ☐ | |
| SE-PB-02 | Decode as mappings: list, Remove mapping, "None." | `components/Settings/ProtobufPage.tsx` | G12 | ☐ | |

### Mocking

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-MO-01 | Recordings: Add from file…, list (name, origin, hosts), Remove <name> | `components/Settings/MockingPage.tsx` | G12 | ☐ | |
| SE-MO-02 | OpenAPI specs: Import…, list (name, servers), Generate mocks (toast "N mock rules added, turned off…"), Remove <name> | `components/Settings/MockingPage.tsx`, `lib/actions.ts` | G12 | ☐ | |

### Appearance

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-AP-01 | Theme: System, Light, Dark radio group | `components/Settings/AppearancePage.tsx` | G12 | ☐ | |
| SE-AP-02 | Flow table columns: Reset columns | `components/Settings/AppearancePage.tsx` | G12 | ☐ | |

### Advanced

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-AD-01 | Breakpoints: Timeout (seconds), Save | `components/Settings/AdvancedPage.tsx` | G12 | ☐ | |
| SE-AD-02 | Scripts: Rewrite and mock scripts, View scripts (ms), Save | `components/Settings/AdvancedPage.tsx` | G12 | ☐ | |
| SE-AD-03 | Upstream connections: Keep idle connections for, Idle connections per host, Connect timeout, Save | `components/Settings/AdvancedPage.tsx` | G12 | ☐ | |

### MCP

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-MCP-01 | MCP server switch with status ("Starting…", URL, error) | `components/Settings/McpPage.tsx` | G12 | ☐ | |
| SE-MCP-02 | Read-only switch | `components/Settings/McpPage.tsx` | G12 | ☐ | |
| SE-MCP-03 | Connect an agent: URL and token copies, Claude Code command, JSON config, `proxymate mcp-stdio`; Regenerate token (Regenerate / Cancel, "Connected agents will need the new token.") | `components/Settings/McpPage.tsx` `ConnectSection` | G12 | ☐ | |
| SE-MCP-04 | Connected clients (n) with menu Copy name, Copy name and version, Copy session id; "No agent is connected." | `components/Settings/McpPage.tsx` `ClientsSection` | G12 | ☐ | |
| SE-MCP-05 | Audit log: Time, Client, Tool, Arguments, Result; Refresh, Clear, Load older calls; menu Copy tool, Copy arguments, Copy error, Copy client, Copy entry as JSON; "No calls yet." | `components/Settings/McpPage.tsx` `AuditSection` | G12 | ☐ | |

### Updates

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| SE-UP-01 | Updates page: "Proxymate <version>" and where updates come from | `components/Settings/UpdatesPage.tsx` | G12 | ☐ | |
| SE-UP-02 | Check Now | `components/Settings/UpdatesPage.tsx` | G15 | ☐ | |
| SE-UP-03 | "Check for updates at launch" switch | `components/Settings/UpdatesPage.tsx` | G15 | ☐ | |

## macOS integration

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| MI-01 | Certificate wizard: checks "Trusted in the login keychain" and "Verified with a real HTTPS handshake" with detail; CA name | `components/CertWizard.tsx` | G13 | ☐ | |
| MI-02 | Wizard buttons: Install & Trust ("Waiting for macOS…"), Check again, Done, Remove from keychain; "Cancelled. Nothing was changed." | `components/CertWizard.tsx` | G13 | ☐ | |
| MI-03 | Wizard: Firefox instructions and Copy terminal env | `components/CertWizard.tsx` | G13 | ☐ | |
| MI-04 | Certificate popover "HTTPS decryption": trust state with Install & Trust… or Manage… | `components/CertificateMenu.tsx` | G13 | ☐ | |
| MI-05 | Certificate popover fields: Root CA, Valid until, SHA-256, PEM file (copy button), Try it curl (copy button) | `components/CertificateMenu.tsx` | G13 | ☐ | |
| MI-06 | CA menu: Copy common name, Copy expiry date, Copy SHA-256, Copy SHA-256 without colons, Copy PEM file path, Copy curl example | `components/CertificateMenu.tsx` | G13 | ☐ | |
| MI-07 | Regenerate… then Replace CA, with "Remove trust for the old CA from this Mac" switch; error line | `components/CertificateMenu.tsx` | G13 | ☐ | |
| MI-08 | System proxy popover: "System proxy on/off", description, Services (Copy services), unsupported note, error, Use as system proxy / Turn off | `components/SystemProxyMenu.tsx` | G13 | ☐ | |
| MI-09 | System proxy popover: Copy terminal env ("Copied") | `components/SystemProxyMenu.tsx` | G13 | ☐ | |
| MI-10 | Previous system proxy settings come back on quit and after a crash | `components/SystemProxyMenu.tsx` | G13 | ☐ | |
| MI-11 | Device setup sheet: tabs iOS Simulator, iPhone, Android, Emulator; "Allow devices on your network" switch, interface, proxy address to copy; Devices on your network with Allow, Deny, Forget | `components/DeviceSetup/DeviceSetupSheet.tsx` | G13 | ☐ | |
| MI-12 | iOS Simulator tab: booted simulators with certificate state, Install / Install again, Install to all booted, system proxy state; "No simulator is booted", simctl missing; Copy name, UDID, runtime | `components/DeviceSetup/SimulatorTab.tsx` | G13 | ☐ | |
| MI-13 | iPhone tab: steps (Wi-Fi, Server and Port to copy, setup page and QR, profile install, certificate trust) and Status | `components/DeviceSetup/PhoneTabs.tsx`, `components/DeviceSetup/parts.tsx` | G13 | ☐ | |
| MI-14 | Android tab: steps, certificate download, network_security_config.xml and manifest attribute to copy, Status | `components/DeviceSetup/PhoneTabs.tsx` | G13 | ☐ | |
| MI-15 | Emulator tab: running emulators, Use / Stop using Proxymate as its proxy, Push certificate, terminal commands to copy; "No emulator is running" | `components/DeviceSetup/EmulatorTab.tsx` | G13 | ☐ | |
| MI-16 | LAN Allow prompt: "Allow <ip>?" (Copy IP address), nickname, "N connections waiting · refused in N s", Deny, Allow; dismissing keeps it waiting | `components/DeviceAllowPrompt.tsx` | G13 | ☐ | |
| MI-17 | Intercept popover: terminals (Terminal, iTerm: "curl, node, python, git…"), browsers with a fresh profile (Chrome, Edge, Brave, Arc, Firefox; "not installed" disabled), Running list with pid | `components/InterceptMenu.tsx` | G13 | ☐ | |
| MI-18 | Intercept launch toast "Started <name>" with notes; intercepted flows named "<App> (Intercepted)"; "Can't start the app" | `lib/actions.ts` `intercept` | G13 | ☐ | |

## Updates

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| UP-01 | Check for Updates…: "You're up to date" or "Can't check for updates" | `src-tauri/src/updater.rs` | G15 | ☐ | |
| UP-02 | "Update available": "Proxymate X is available. You have Y." with Install and Restart / Later | `src-tauri/src/updater.rs` | G15 | ☐ | |
| UP-03 | "Can't install the update" on failure | `src-tauri/src/updater.rs` | G15 | ☐ | |
| UP-04 | Quiet check at launch when enabled; only speaks when an update exists | `src-tauri/src/updater.rs` | G15 | ☐ | |

## Dev tooling

| ID | Behaviour | Source | Phase | Status | Evidence |
|---|---|---|---|---|---|
| DV-01 | Dev remote: `theme` | `dev/remote.ts` | G0 | ☑ | G0: apps/native/src/dev/mod.rs |
| DV-02 | Dev remote: `click` by test id | `dev/remote.ts` | G2 | ☑ | G2: G0's `click`, now on every chrome control (`.test_support()` on plain elements); `tests/shell.rs` clicks chips, rows and tabs |
| DV-03 | Dev remote: `select`, `selectIds`, `selectAll`, `scrollTo` | `dev/remote.ts` | G1 | ☑ | G1: `apps/native/src/dev/mod.rs` `select`, `selectIds`, `selectAll`, `scrollTo` (plus `headerColumns`, `filter`, `autoSelect`, `table`) |
| DV-04 | Dev remote: `filter`, `palette`, `store`, `openDocument`, `closeDocument`, `saveSession`, `exportHar`, `contextMenu`, `reload` | `dev/remote.ts` | G2 | ☑ | G2: `apps/native/src/dev/mod.rs` `filter`, `search`, `palette`, `command`, `keys`, `store`, `openDocument`, `closeDocument`, `saveSession`, `exportHar`, `contextMenu` (with `run`), `workspace`, … (`reload` has no meaning without a web view) |
| DV-05 | Dev remote: `requestTab`, `responseTab`, `curl` | `dev/remote.ts` | G3 | ☐ | |
| DV-06 | Dev remote: `viewer` | `dev/remote.ts` | G4 | ☐ | |
| DV-07 | Dev remote: `wsSend`, `wsClose` | `dev/remote.ts` | G6 | ☐ | |
| DV-08 | Dev remote: `rules`, `rulesEditor`, `preset`, `saveRule`, `testRule`, `deleteRule`, `throttle`, `noCache`, `noCookies` | `dev/remote.ts` | G7 | ☐ | |
| DV-09 | Dev remote: `replay`, `editReplay`, `compose`, `composeSend`, `breakpoints`, `breakpointAction` | `dev/remote.ts` | G8 | ☐ | |
| DV-10 | Dev remote: `rewrite`, `rewriteEdit` | `dev/remote.ts` | G9 | ☐ | |
| DV-11 | Dev remote: `scripts`, `scriptTemplate`, `scriptSource`, `saveScript`, `testScript` | `dev/remote.ts` | G10 | ☐ | |
| DV-12 | Dev remote: `label`, `comment`, `headerColumns`, `compare`, `repeat`, `code`, `deleteSelection` | `dev/remote.ts` | G11 | ☐ | |
| DV-13 | Dev remote: `settings`, `appearance`, `mockFrom`, `mockFromFile`, `importOpenApi`, `generateMocks`, `removeMockSource`, `removeOpenApi`, `importProto`, `removeProto`, `neverRecord`, `recordingLists`, `autosave`, `autosaveNow` | `dev/remote.ts` | G12 | ☐ | |
| DV-14 | Dev remote: `intercept`, `deviceTab` | `dev/remote.ts` | G13 | ☐ | |
| DV-15 | `VITE_PM_AUTO_SELECT`: start with Auto Select on | `dev/harness.ts` | G1 | ☑ | G1: `PROXYMATE_DEV_AUTO_SELECT=1` (`features/flow_table.rs`, `dev_env.rs`) |
| DV-16 | Perf probe (`PROXYMATE_SYNTH_FLOWS` + `VITE_PM_PERF_PROBE`): `[perf-probe]` scroll and filter timings | `dev/perfProbe.ts` | G1 | ☑ | G1: `apps/native/src/perf_probe.rs`, `PROXYMATE_DEV_PERF_PROBE=1`; numbers in the G1 PR |
| DV-17 | `VITE_PM_FILTER`: start with search text | `dev/harness.ts` | G2 | ☑ | G2: `PROXYMATE_DEV_FILTER` (`dev_env::value`, `Workspace::new`) |
| DV-18 | `VITE_PM_RESPONSE_TAB`: open the response pane on a tab | `dev/harness.ts` | G3 | ☐ | |
| DV-19 | `VITE_PM_OPEN=rules` | `dev/harness.ts` | G7 | ☐ | |
| DV-20 | `VITE_PM_OPEN=breakpoints` | `dev/harness.ts` | G8 | ☐ | |
| DV-21 | `VITE_PM_OPEN=scripts` | `dev/harness.ts` | G10 | ☐ | |
| DV-22 | `VITE_PM_OPEN=settings:<page>` | `dev/harness.ts` | G12 | ☐ | |
| DV-23 | `VITE_PM_OPEN=cert-wizard` and `device-setup:<tab>` | `dev/harness.ts` | G13 | ☐ | |
| DV-24 | Body-viewer zoo: one flow per fixture in `crates/pm-testkit/fixtures/zoo/` for viewer screenshots | `dev/zoo.ts` | G4 | ☐ | |
| DV-25 | Browser-only harness: mock backend and URL parameters (`?zoo`, `?select`, `?requestTab`, `?rules`, `?rewrite`, `?scripts`, `?theme`, `?mocks`, `?devicePrompt`…) | `dev/harness.ts`, `lib/mockBackend.ts` | G14 | ☐ | |

## Summary

Rows per owning phase. G14 also audits every row; its own row is the browser-only harness, expected to be signed off as a deviation.

| Phase | Rows | Done (`☑`) |
|---|---|---|
| G0 | 14 | 14 |
| G1 | 39 | 35 |
| G2 | 91 | 89 (2 `≈`: WM-13, WM-28) |
| G3 | 62 | 0 |
| G4 | 31 | 0 |
| G5 | 10 | 0 |
| G6 | 19 | 0 |
| G7 | 60 | 0 |
| G8 | 32 | 0 |
| G9 | 19 | 0 |
| G10 | 23 | 0 |
| G11 | 36 | 0 |
| G12 | 51 | 0 |
| G13 | 29 | 0 |
| G14 | 1 | 0 |
| G15 | 7 | 0 |
| **Total** | **524** | **138** |
