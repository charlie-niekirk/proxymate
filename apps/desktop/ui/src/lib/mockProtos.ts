import type { BodyPart, DecodedBody_Serialize, ProtoMapping_Serialize, ProtoSchema } from "@/bindings";
import type { Backend } from "@/lib/backend";

type ProtoMethods = Pick<
  Backend,
  | "decodeBody"
  | "listProtoSchemas"
  | "importProtoSchema"
  | "removeProtoSchema"
  | "listProtoMessageTypes"
  | "listProtoMappings"
  | "setProtoMappings"
>;

/** The testkit's `shop.proto`, as the mock "imports" it. */
const SHOP: Omit<ProtoSchema, "id" | "added_at_ms"> = {
  name: "shop",
  files: ["shop.proto"],
  services: ["shop.v1.Orders"],
  messages: 4,
};
const SHOP_TYPES = ["shop.v1.GetOrderRequest", "shop.v1.LineItem", "shop.v1.ListOrdersRequest", "shop.v1.Order"];

/**
 * Protobuf in the browser mock. Nothing decodes here: zoo flows answer with
 * the engine's own output (`<fixture>.<part>.decoded.json`, written by
 * `crates/pm-core/tests/zoo.rs`), and an import adds the testkit's schema
 * by name. `?protos` starts with it imported.
 */
export function createMockProtos(
  decoded: (store: number, id: number) => Partial<Record<BodyPart, DecodedBody_Serialize>> | undefined,
): ProtoMethods {
  const preloaded = typeof window !== "undefined" && new URLSearchParams(window.location.search).has("protos");
  let schemas: ProtoSchema[] = preloaded ? [{ ...SHOP, id: 1, added_at_ms: Date.now() }] : [];
  let mappings: ProtoMapping_Serialize[] = [];
  let next = schemas.length + 1;
  return {
    async decodeBody(store, flow, part) {
      const body = decoded(store, flow)?.[part];
      if (!body) throw new Error("this body has no decoded form (mock)");
      return body;
    },
    async listProtoSchemas() {
      return schemas;
    },
    async importProtoSchema() {
      const schema = { ...SHOP, id: next++, added_at_ms: Date.now() };
      schemas = [...schemas, schema];
      return schema;
    },
    async removeProtoSchema(id) {
      schemas = schemas.filter((s) => s.id !== id);
    },
    async listProtoMessageTypes() {
      return schemas.length > 0 ? SHOP_TYPES : [];
    },
    async listProtoMappings() {
      return mappings;
    },
    async setProtoMappings(list) {
      mappings = list.map((m) => ({ ...m, part: m.part ?? null }) as ProtoMapping_Serialize);
    },
  };
}
