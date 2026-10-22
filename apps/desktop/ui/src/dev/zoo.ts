import type { BodyInfo_Serialize, ContentKind, FlowRow_Serialize } from "@/bindings";
import { sniff } from "@/lib/body/content";
import type { MockFlow } from "@/lib/mockBackend";

/**
 * Dev only: the body-viewer zoo (`crates/pm-testkit/fixtures/zoo`) as mock
 * flows, so `npm run dev` with `?zoo` (and the Playwright viewer tests) show
 * the same fixtures the Rust zoo upstream serves.
 */

interface Fixture {
  name: string;
  file?: string;
  generate?: string;
  type?: string;
  encoding?: string;
  method?: string;
  request?: { file: string; type: string };
  status?: number;
  kind: ContentKind;
  graphql_op?: string;
}

/** A fixture file, served raw by the dev server (see `zooFixtures` in vite.config.ts). */
async function file(name: string): Promise<Uint8Array> {
  const res = await fetch(`/__pm_zoo/${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`no zoo file ${name}`);
  return new Uint8Array(await res.arrayBuffer());
}

/** Mirrors `pm_testkit::zoo::big_json`. */
export function bigJson(minLen: number): Uint8Array {
  const parts: string[] = ['{"generated":"proxymate zoo","items":['];
  let len = parts[0]!.length;
  let i = 0;
  while (len < minLen) {
    const item =
      `${i > 0 ? "," : ""}{"id":${i},"name":"item-${i}","active":${i % 3 === 0},"price":${i % 1000}.${String(i % 100).padStart(2, "0")},` +
      `"tags":["t${i % 7}","t${i % 11}"],"owner":{"id":${i % 5000},"login":"user${i % 5000}"},"note":null}`;
    parts.push(item);
    len += item.length;
    i++;
  }
  parts.push(`],"count":${i}}`);
  return new TextEncoder().encode(parts.join(""));
}

async function gzip(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([bytes.slice()]).stream().pipeThrough(new CompressionStream("gzip"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

function info(raw: Uint8Array, kind: ContentKind, mime: string | null, encoding: string | null): BodyInfo_Serialize {
  return { size: raw.length, captured: raw.length, truncated: false, encoding, complete: true, kind, mime };
}

export async function loadZoo(): Promise<MockFlow[]> {
  const manifest = JSON.parse(new TextDecoder().decode(await file("zoo.json"))) as { fixtures: Fixture[] };
  const start = Date.now() - manifest.fixtures.length * 1000;
  return Promise.all(
    manifest.fixtures.map(async (f, id): Promise<MockFlow> => {
      const plain = f.file ? await file(f.file) : bigJson(10 * 1024 * 1024);
      const raw = f.encoding === "gzip" ? await gzip(plain) : plain;
      const request = f.request ? await file(f.request.file) : new Uint8Array();
      const response = sniff(plain, f.type ?? null);
      const requestKind = f.kind === "graphql" ? "graphql" : f.request ? sniff(request, f.request.type).kind : "none";
      const method = f.method ?? "GET";
      const row: FlowRow_Serialize = [
        id, "completed", "https", method, "zoo.proxymate.test", 443, `/zoo/${f.name}`, "", f.status ?? 200, f.kind, "Zoo",
        start + id * 1000, 12 + id, request.length, raw.length, false, "HTTP/2", "intercepted", f.graphql_op ?? null, null,
      ];
      const headers = (type: string | undefined, length: number, encoding?: string) => [
        ...(type ? [{ name: "content-type", value: type }] : []),
        ...(encoding ? [{ name: "content-encoding", value: encoding }] : []),
        { name: "content-length", value: String(length) },
      ];
      return {
        row,
        detail: {
          id,
          request: {
            method,
            url: `https://zoo.proxymate.test/zoo/${f.name}`,
            version: "HTTP/2",
            headers: [
              { name: ":authority", value: "zoo.proxymate.test" },
              { name: "user-agent", value: "proxymate-zoo/1" },
              ...(f.request ? headers(f.request.type, request.length) : []),
            ],
          },
          response: {
            status: f.status ?? 200,
            reason: f.status === 201 ? "Created" : "OK",
            version: "HTTP/2",
            headers: headers(f.type, raw.length, f.encoding),
          },
          request_body: info(request, requestKind, f.request ? sniff(request, f.request.type).mime : null, null),
          response_body: info(raw, response.kind, response.mime, f.encoding ?? null),
          error: null,
          tls: null,
          timing: { dns_ms: 1.2, connect_ms: 3.4, tls_ms: 8.9, ttfb_ms: 20, transfer_ms: 4, reused_connection: false },
          rules: { applied: [], rewrite_skipped: null, breakpoint_timeout: false, mocked: false },
        },
        bodies: {
          request: { raw: request, decoded: request },
          response: { raw, decoded: plain },
        },
      };
    }),
  );
}
