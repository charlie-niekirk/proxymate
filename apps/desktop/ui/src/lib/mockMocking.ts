import type { MockSource, OpenApiSpec, OperationMatch_Serialize, Rule_Serialize } from "@/bindings";
import type { Backend } from "@/lib/backend";
import { emptyMatcher } from "@/lib/rules";

type MockingMethods = Pick<
  Backend,
  | "listMockSources"
  | "createMockSource"
  | "createMockSourceFromFile"
  | "removeMockSource"
  | "draftServeFromRule"
  | "listOpenApiSpecs"
  | "importOpenApiSpec"
  | "removeOpenApiSpec"
  | "generateOpenApiMocks"
  | "flowOperation"
>;

interface Operation {
  method: string;
  path: string;
  operation_id: string;
  summary: string;
  body: string;
}

/** The spec the mock "imports": the mock data's `/v1/items/{n}` traffic. */
const ITEMS: Omit<OpenApiSpec, "id" | "added_at_ms"> = {
  name: "Items API",
  version: "1.4.0",
  openapi: "3.1.0",
  operations: 2,
  servers: ["https://api.example.com"],
};
const ITEM_OPS: Operation[] = [
  { method: "GET", path: "/v1/items/{itemId}", operation_id: "getItem", summary: "Fetch an item", body: '{\n  "id": 0,\n  "name": "string"\n}' },
  { method: "POST", path: "/v1/items", operation_id: "createItem", summary: "Create an item", body: "" },
];

function templateRegex(path: string): RegExp {
  const source = path
    .split(/(\{[^}]+\})/)
    .map((part) => (part.startsWith("{") ? "[^/]+" : part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")))
    .join("");
  return new RegExp(`^${source}/?$`);
}

/**
 * Offline mocking in the browser mock (M20): a library entry per "Mock from
 * session…", and one canned OpenAPI spec over the mock data's paths.
 * `?openapi` starts with it imported, `?mocks` with a library entry.
 */
export function createMockMocking(deps: {
  documentName: (store: number) => string | undefined;
  hostsOf: (store: number) => string[];
  flowOf: (store: number, id: number) => { method: string; path: string } | undefined;
  saveRules: (rules: Rule_Serialize[]) => Promise<Rule_Serialize[]>;
  usedBy: (source: string) => string[];
}): MockingMethods {
  const params = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : new URLSearchParams();
  const now = Date.now();
  let specs: OpenApiSpec[] = params.has("openapi") ? [{ ...ITEMS, id: 1, added_at_ms: now }] : [];
  let nextSpec = specs.length + 1;
  const entry = (name: string, origin: string, hosts: string[]): MockSource => ({
    id: crypto.randomUUID(),
    name,
    origin,
    flows: 40,
    hosts,
    created_at_ms: Date.now(),
    size: 184_320,
  });
  let sources: MockSource[] = params.has("mocks")
    ? [entry("checkout-recording", "/Users/you/Documents/checkout-recording.proxymate", ["api.example.com", "cdn.example.com"])]
    : [];
  return {
    async listMockSources() {
      return sources;
    },
    async createMockSource(store, name) {
      const doc = deps.documentName(store);
      const source = entry(name ?? doc?.replace(/\.[^.]+$/, "") ?? "Live capture", doc ? `/Users/you/Documents/${doc}` : "Live capture", deps.hostsOf(store));
      sources = [...sources, source];
      return source;
    },
    async createMockSourceFromFile(path) {
      const file = path ?? "/Users/you/Documents/recording.proxymate";
      const source = entry(file.split("/").pop()!.replace(/\.[^.]+$/, ""), file, ["api.example.com"]);
      sources = [...sources, source];
      return source;
    },
    async removeMockSource(id) {
      const users = deps.usedBy(id);
      if (users.length) throw new Error(`the mock source is used by ${users.map((n) => JSON.stringify(n)).join(", ")}; delete or change it first`);
      sources = sources.filter((s) => s.id !== id);
    },
    async draftServeFromRule(id) {
      const source = sources.find((s) => s.id === id);
      if (!source) throw new Error(`there is no mock source with id ${id}`);
      const host =
        source.hosts.length === 0
          ? null
          : source.hosts.length === 1
            ? { kind: "exact" as const, value: source.hosts[0]!, case_insensitive: false }
            : { kind: "glob" as const, value: `{${source.hosts.join(",")}}`, case_insensitive: true };
      return {
        id: "",
        name: `Serve from ${source.name}`,
        enabled: true,
        order: 0,
        origin: { kind: "preset", preset: "serve_from" },
        matcher: { ...emptyMatcher(), host },
        actions: [{ type: "ServeFrom", source: id, key: { query: true, body: false }, on_miss: "pass_through", pick: "first" }],
      };
    },
    async listOpenApiSpecs() {
      return specs;
    },
    async importOpenApiSpec() {
      const spec = { ...ITEMS, id: nextSpec++, added_at_ms: Date.now() };
      specs = [...specs, spec];
      return spec;
    },
    async removeOpenApiSpec(id) {
      specs = specs.filter((s) => s.id !== id);
    },
    async generateOpenApiMocks(id) {
      const spec = specs.find((s) => s.id === id);
      if (!spec) throw new Error(`there is no OpenAPI spec ${id}`);
      return deps.saveRules(
        ITEM_OPS.map((op) => ({
          id: "",
          name: `${op.operation_id} · ${spec.name}`,
          enabled: false,
          order: 0,
          origin: { kind: "preset", preset: "open_api" },
          matcher: {
            ...emptyMatcher(),
            methods: [op.method],
            host: { kind: "exact", value: "api.example.com", case_insensitive: false },
            path: { kind: "regex", value: templateRegex(op.path).source, case_insensitive: false },
          },
          actions: [
            {
              type: "MapLocal",
              source: { kind: "inline", body: op.body },
              status: op.method === "POST" ? 201 : 200,
              headers: [],
              content_type: op.body ? "application/json" : null,
            },
          ],
        })),
      );
    },
    async flowOperation(store, id): Promise<OperationMatch_Serialize | null> {
      const spec = specs[0];
      const flow = deps.flowOf(store, id);
      if (!spec || !flow) return null;
      const op = ITEM_OPS.find((o) => o.method === flow.method && templateRegex(o.path).test(flow.path));
      return op
        ? { spec_id: spec.id, spec_name: spec.name, operation_id: op.operation_id, summary: op.summary, method: op.method, path: op.path }
        : null;
    },
  };
}
