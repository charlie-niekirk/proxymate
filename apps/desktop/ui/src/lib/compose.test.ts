import { describe, expect, it } from "vitest";

import { contentTypeOf, draftOf, languageFor, newDraft, specOf, withContentType } from "./compose";

describe("compose", () => {
  it("sets, replaces and removes the content type in place", () => {
    const headers = [
      { name: "accept", value: "*/*" },
      { name: "Content-Type", value: "text/plain" },
    ];
    expect(withContentType(headers, "application/json")).toEqual([
      { name: "accept", value: "*/*" },
      { name: "Content-Type", value: "application/json" },
    ]);
    expect(withContentType(headers, null)).toEqual([{ name: "accept", value: "*/*" }]);
    expect(withContentType([], "text/plain")).toEqual([{ name: "content-type", value: "text/plain" }]);
    expect(contentTypeOf(headers)).toBe("text/plain");
  });

  it("picks the editor language from the content type", () => {
    expect(languageFor("application/json; charset=utf-8")).toBe("json");
    expect(languageFor("application/vnd.api+json")).toBe("json");
    expect(languageFor("application/soap+xml")).toBe("xml");
    expect(languageFor("application/graphql")).toBe("graphql");
    expect(languageFor(null)).toBe("text");
  });

  it("turns a draft into a spec", () => {
    const draft = { ...newDraft(), method: " post ", headers: [{ name: " ", value: "x" }, { name: "x-a", value: "1" }] };
    expect(specOf(draft)).toEqual({
      method: "POST",
      url: "https://",
      headers: [{ name: "x-a", value: "1" }],
      body: null,
      body_base64: false,
    });
    const binary = draftOf({ method: "PUT", url: "http://a/", body: "AAE=", body_base64: true });
    expect(specOf(binary)).toMatchObject({ body: "AAE=", body_base64: true, headers: [] });
  });
});
