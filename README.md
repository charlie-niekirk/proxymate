# Proxymate

A fast desktop HTTP(S) debugging proxy, built with Tauri 2, a Rust core and a React UI. macOS comes first, then Linux and Windows.

> Early development. M0 (the skeleton) shows 100k synthetic flows through the real engine → IPC → virtualised table path. The proxy itself lands in M1. See [`docs/PLAN.md`](docs/PLAN.md) for the full plan and milestones.

![Proxymate M0](docs/screenshots/m0-skeleton.png)

## Develop

Requirements:
- Rust (stable, edition 2024).
- Node 22 or newer, with npm.
- The [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run tauri dev
```

See [`CLAUDE.md`](CLAUDE.md) for all commands, the crate layout and conventions.
