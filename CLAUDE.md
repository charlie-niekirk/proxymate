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
| Run the app with the perf probe | `VITE_PM_PERF_PROBE=1 npm run tauri dev`. Scroll and filter timings print in the terminal as `[perf-probe]` |
| UI only, in a browser | `npm run dev` (http://localhost:1420, in-memory mock backend) |
| Rust format / lint / test | `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` |
| UI typecheck / lint / test / build | `npm run typecheck`, `npm run lint`, `npm run test`, `npm run build` |
| Regenerate TS bindings | `cargo test -p proxymate-desktop export_bindings`. Debug app launches also regenerate them |
| Engine micro-bench | `cargo run --release -p proxymate-cli -- bench [FLOWS]` |
| Release bundle | `npm run tauri build` |

CI (`.github/workflows/ci.yml`):
- Rust: `fmt --check`, `clippy -D warnings`, `test` and a bindings-drift check, on macOS and Linux.
- UI: `typecheck`, `lint`, `vitest` and `build`.

Run the same checks locally before pushing.

## Layout and crate boundaries

```
crates/pm-model      pure types (serde + specta): FlowSummary, FlowRow, FilterSpec, SortSpec, ViewDelta, ...
crates/pm-core       headless Engine: store/ (arena + views), synth (synthetic flows); later proxy/, tls/, rules/, io/
crates/pm-platform   OS integration traits + cfg impls (sysproxy, trust, procinfo, ...), empty until M4
crates/pm-mcp        MCP server over Arc<Engine>, empty until M7
crates/pm-testkit    test upstreams, test CA, re-export of pm_core::synth
apps/desktop/src-tauri   thin Tauri adapters (commands.rs) + window chrome (window.rs)
apps/desktop/ui          Vite + React 19 + TS strict + Tailwind v4 + shadcn/ui
apps/cli                 proxymate-cli (headless; `bench` today)
```

Rules:
- **Adapters call only the `Engine` facade** (`pm_core::Engine`). Tauri commands, MCP tools and the CLI never reach into `pm_core::store` or other internals. This keeps the UI and agents behaving the same. If an adapter needs something new, add it to `Engine`.
- `pm-core` has **no Tauri dependency** and stays headless and testable.
- `pm-model` holds data types only: no I/O and no engine logic. Every type that crosses IPC lives here and derives `Serialize`, `Deserialize` and `specta::Type`.
- OS-specific code goes in `pm-platform` behind traits, never `cfg`-sprinkled through `pm-core`.

## Data path (performance core)

1. Flows enter the store in batches, under one write lock per batch (`Engine::insert_flows` / `update_flows`). Views update incrementally.
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

**The flow table**
- It is a custom div grid virtualised with TanStack Virtual, at a fixed 22 px row height. Do not swap in shadcn's DataTable.
- Keep row components memoised, with stable callbacks.

**Window chrome**
- The overlay title bar with inset traffic lights is set in `tauri.conf.json`.
- The window is transparent and `window.rs` applies sidebar vibrancy. Panes must paint opaque backgrounds, except the sidebar (`--sidebar` is transparent on macOS).

**Synthetic data**
- Until the proxy exists, the app seeds 100k synthetic flows plus a live trickle.
- Tune it with `PROXYMATE_SYNTH_FLOWS` and `PROXYMATE_SYNTH_RATE`; set the rate to 0 to stop the trickle.
- M1 replaces the seeding with real capture. Keep `synth` for tests and benches.
