import { describe, expect, it } from "vitest";

import { parseGraphQlBody, parseGraphQlQueryString } from "./graphql";

describe("GraphQL parsing", () => {
  it("reads a JSON envelope", () => {
    const ops = parseGraphQlBody('{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","variables":{"id":"1"}}', "application/json");
    expect(ops).toEqual([
      { operationName: "GetUser", type: "query", query: "query GetUser($id: ID!) { user(id: $id) { name } }", variables: { id: "1" }, extensions: null },
    ]);
  });

  it("reads batches, raw documents and persisted queries", () => {
    expect(parseGraphQlBody('[{"query":"mutation A { a }"},{"query":"{ b }"}]', null)?.map((o) => [o.operationName, o.type])).toEqual([
      ["A", "mutation"],
      [null, "query"],
    ]);
    expect(parseGraphQlBody("subscription S { s }", "application/graphql")?.[0]?.operationName).toBe("S");
    const persisted = parseGraphQlBody('{"operationName":"P","extensions":{"persistedQuery":{"version":1}}}', null);
    expect(persisted?.[0]).toMatchObject({ operationName: "P", query: "", type: null });
  });

  it("rejects other JSON", () => {
    expect(parseGraphQlBody('{"q":1}', "application/json")).toBeNull();
    expect(parseGraphQlBody("not json", "application/json")).toBeNull();
  });

  it("reads GET operations", () => {
    const op = parseGraphQlQueryString("https://x/graphql?query=%7B%20me%20%7B%20id%20%7D%20%7D&variables=%7B%22a%22%3A1%7D&operationName=Me");
    expect(op).toMatchObject({ operationName: "Me", query: "{ me { id } }", variables: { a: 1 } });
    expect(parseGraphQlQueryString("https://x/graphql")).toBeNull();
  });
});
