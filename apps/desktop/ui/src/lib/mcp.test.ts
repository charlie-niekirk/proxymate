import { describe, expect, it } from "vitest";

import { claudeAddCommand, maskToken, mcpJsonConfig } from "@/lib/mcp";

describe("MCP connection snippets", () => {
  const url = "http://127.0.0.1:9095/mcp";

  it("builds the claude mcp add command with the bearer header", () => {
    expect(claudeAddCommand(url, "abc")).toBe(
      'claude mcp add --transport http proxymate http://127.0.0.1:9095/mcp --header "Authorization: Bearer abc"',
    );
  });

  it("builds an mcpServers JSON entry", () => {
    const parsed = JSON.parse(mcpJsonConfig(url, "abc")) as {
      mcpServers: { proxymate: { type: string; url: string; headers: Record<string, string> } };
    };
    expect(parsed.mcpServers.proxymate).toEqual({ type: "http", url, headers: { Authorization: "Bearer abc" } });
  });

  it("masks all but the end of the token", () => {
    expect(maskToken("0123456789abcdef")).toBe("••••••••cdef");
    expect(maskToken("ab")).toBe("••••");
  });
});
