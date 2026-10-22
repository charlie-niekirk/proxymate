/** How agents connect to the MCP endpoint: the snippets Settings › MCP copies. */

/** `claude mcp add` for Claude Code. */
export function claudeAddCommand(url: string, token: string): string {
  return `claude mcp add --transport http proxymate ${url} --header "Authorization: Bearer ${token}"`;
}

/** The `mcpServers` entry most clients (Cursor, Claude Desktop, `.mcp.json`) read. */
export function mcpJsonConfig(url: string, token: string): string {
  const config = {
    mcpServers: {
      proxymate: { type: "http", url, headers: { Authorization: `Bearer ${token}` } },
    },
  };
  return JSON.stringify(config, null, 2);
}

/** The token with all but its last four characters hidden, for display. */
export function maskToken(token: string): string {
  return token.length <= 4 ? "••••" : `${"•".repeat(8)}${token.slice(-4)}`;
}
