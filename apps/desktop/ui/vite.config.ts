import { readFile } from "node:fs/promises";
import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";

const host = process.env.TAURI_DEV_HOST;

/** Dev server endpoint that prints reports from src/dev/perfProbe.ts. */
function perfProbeSink(): Plugin {
  return {
    name: "pm-perf-probe-sink",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__pm_perf", (req, res) => {
        let body = "";
        req.on("data", (chunk: Buffer) => (body += chunk));
        req.on("end", () => {
          console.log(`\n[perf-probe] ${body}\n`);
          res.end("ok");
        });
      });
    },
  };
}

/**
 * Dev server mailbox for driving the app from a shell (`VITE_PM_REMOTE=1`,
 * see src/dev/remote.ts): `curl -d '{"select":3}' localhost:1420/__pm_remote`
 * queues a command, which the webview picks up on its next poll.
 */
function remoteControl(): Plugin {
  const queue: string[] = [];
  return {
    name: "pm-remote-control",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__pm_remote", (req, res) => {
        if (req.method === "POST") {
          let body = "";
          req.on("data", (chunk: Buffer) => (body += chunk));
          req.on("end", () => {
            queue.push(body);
            res.end("queued\n");
          });
          return;
        }
        const next = queue.shift();
        res.statusCode = next ? 200 : 204;
        res.end(next ?? "");
      });
    },
  };
}

/**
 * Dev server endpoint serving the body-viewer zoo fixtures byte for byte
 * (`/__pm_zoo/<file>`), for the `?zoo` mock backend (src/dev/zoo.ts).
 * Vite's own asset URLs would serve .css and .js files as modules.
 */
function zooFixtures(): Plugin {
  const dir = path.resolve(import.meta.dirname, "../../../crates/pm-testkit/fixtures/zoo");
  return {
    name: "pm-zoo-fixtures",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__pm_zoo/", (req, res) => {
        const name = decodeURIComponent((req.url ?? "").replace(/^\//, "").split("?")[0]!);
        if (!/^[\w.-]+$/.test(name)) {
          res.statusCode = 400;
          res.end();
          return;
        }
        readFile(path.join(dir, name)).then(
          (bytes) => {
            res.setHeader("Content-Type", "application/octet-stream");
            res.end(bytes);
          },
          () => {
            res.statusCode = 404;
            res.end();
          },
        );
      });
    },
  };
}

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react(), tailwindcss(), perfProbeSink(), zooFixtures(), remoteControl()],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "./src") },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari15",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
