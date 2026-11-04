import { describe, expect, it } from "vitest";

import type { FilterSpec_Deserialize, FlowSummary_Serialize, SidebarSnapshot_Deserialize } from "@/bindings";
import { decodeRow, type Row } from "@/lib/flow";

import { createMockBackend } from "./mockBackend";

/** A summary as the table's `Row`, by field name: what `decodeRow` must make of the same flow's tuple. */
function summaryRow(s: FlowSummary_Serialize): Row {
  return {
    id: s.id,
    state: s.state,
    scheme: s.scheme,
    method: s.method,
    host: s.host,
    port: s.port,
    path: s.path,
    query: s.query,
    status: s.status,
    contentKind: s.content_kind,
    client: s.client,
    startedAtMs: s.started_at_ms,
    durationMs: s.duration_ms,
    reqSize: s.req_size,
    respSize: s.resp_size,
    edited: s.edited,
    httpVersion: s.http_version,
    tls: s.tls,
    graphqlOp: s.graphql_op,
    sentBy: s.sent_by,
    pinned: s.pinned,
    grpcStatus: s.grpc_status,
    label: s.label ?? null,
    comment: s.comment ?? null,
    extra: s.extra ?? [],
  };
}

const all: FilterSpec_Deserialize = { text: "", protocol: "all", content: [], host: null, client: null };

describe("mock backend", () => {
  it("pushes a sidebar that adds up", async () => {
    const backend = createMockBackend(1_000);
    const snapshot = await new Promise<SidebarSnapshot_Deserialize>((resolve) => void backend.subscribeSidebar(resolve));
    expect(snapshot.total).toBe(1_000);
    expect(snapshot.apps.reduce((n, a) => n + a.count, 0)).toBe(1_000);
    expect(snapshot.domains.reduce((n, d) => n + d.count, 0)).toBe(1_000);
    expect(snapshot.domains.find((d) => d.domain === "github.com")?.hosts).toEqual([{ host: "api.github.com", count: 167 }]);
  });

  it("filters by app and by domain like the engine", async () => {
    const backend = createMockBackend(1_000);
    const safari = await backend.createView(0, { ...all, client: "Safari" }, { key: "id", dir: "asc" });
    expect(safari.len).toBe(200);
    const github = await backend.createView(0, { ...all, host: "*.github.com" }, { key: "id", dir: "asc" });
    const exact = await backend.createView(0, { ...all, host: "api.github.com" }, { key: "id", dir: "asc" });
    expect(github.len).toBe(exact.len);
    expect(github.len).toBeGreaterThan(0);
  });

  // The mock builds `getFlow`'s summary from its row tuples by position.
  // This ties those positions to `decodeRow`'s, so a FlowRow change that
  // updates one and not the other fails here instead of in the inspector.
  it("serves the same flow from getFlow and getRows", async () => {
    const backend = createMockBackend(300);
    await backend.labelFlows(0, { kind: "ids", ids: [5] }, "purple");
    await backend.commentFlow(0, 5, "a note");
    await backend.pinFlows(0, { kind: "ids", ids: [5] }, true);
    const view = await backend.createView(0, all, { key: "id", dir: "asc" });
    const page = await backend.getRows(view.view, 0, 300);
    expect(page.rows).toHaveLength(300);
    for (const tuple of page.rows) {
      const row = decodeRow(tuple);
      const summary = await backend.getFlow(0, row.id);
      expect(summary, `flow ${row.id}`).not.toBeNull();
      expect(summaryRow(summary!)).toEqual(row);
    }
    expect(decodeRow(page.rows[5]!)).toMatchObject({ label: "purple", comment: "a note", pinned: true });
  });

  it("keeps a document's flows to its own store", async () => {
    const backend = createMockBackend(300);
    const doc = (await backend.openDocument(null, false))!;
    expect(doc.flows).toBe(50);
    expect(await backend.getFlow(doc.store, 3)).not.toBeNull();
    expect(await backend.getFlow(doc.store, 120)).toBeNull();
    expect(await backend.getFlowDetail(doc.store, 120)).toBeNull();
    expect(await backend.getFlow(0, 120)).not.toBeNull();
  });

  it("excludes apps from decryption case-insensitively", async () => {
    const backend = createMockBackend(10);
    expect((await backend.setAppSslExcluded("com.apple.Safari", true)).exclude_apps).toEqual(["com.apple.Safari"]);
    expect((await backend.setAppSslExcluded("COM.APPLE.SAFARI", false)).exclude_apps).toEqual([]);
  });
});
