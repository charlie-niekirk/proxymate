import { describe, expect, it } from "vitest";

import { formatMarkup } from "./markup";

describe("formatMarkup", () => {
  it("indents nested XML and keeps short text inline", () => {
    const xml = '<?xml version="1.0"?><feed><title>Hi &amp; bye</title><entry id="1"><a/><b></b></entry><!-- c --></feed>';
    expect(formatMarkup(xml)).toBe(
      ['<?xml version="1.0"?>', "<feed>", "  <title>Hi &amp; bye</title>", '  <entry id="1">', "    <a/>", "    <b></b>", "  </entry>", "  <!-- c -->", "</feed>"].join("\n"),
    );
  });

  it("knows HTML void and raw-text elements", () => {
    const html = "<!DOCTYPE html><html><head><meta charset=utf-8><script>if (a < b) { go(); }</script></head><body><p>x<br>y</p></body></html>";
    expect(formatMarkup(html, { html: true })).toBe(
      [
        "<!DOCTYPE html>",
        "<html>",
        "  <head>",
        "    <meta charset=utf-8>",
        "    <script>if (a < b) { go(); }</script>",
        "  </head>",
        "  <body>",
        "    <p>",
        "      x",
        "      <br>",
        "      y",
        "    </p>",
        "  </body>",
        "</html>",
      ].join("\n"),
    );
  });

  it("keeps multi-line scripts verbatim but re-indented", () => {
    const out = formatMarkup("<div><style>\n    a { b: c }\n      d { e: f }\n</style></div>", { html: true });
    expect(out).toBe(["<div>", "  <style>", "    a { b: c }", "      d { e: f }", "  </style>", "</div>"].join("\n"));
  });

  it("survives unbalanced and truncated markup", () => {
    expect(formatMarkup("<a><b></a><c")).toBe(["<a>", "  <b>", "</a>", "<c"].join("\n"));
  });
});
