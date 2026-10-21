import { describe, expect, it } from "vitest";

import { previewDocument } from "./htmlPreview";

const parse = (html: string) => new DOMParser().parseFromString(html, "text/html");

describe("previewDocument", () => {
  it("puts the CSP first in the head and keeps the doctype", () => {
    const out = previewDocument("<!DOCTYPE html><html><head><title>t</title></head><body>x</body></html>");
    expect(out.startsWith("<!DOCTYPE html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'")).toBe(true);
    expect(parse(out).title).toBe("t");
  });

  it("keeps quirks-mode documents without a doctype", () => {
    expect(previewDocument("<p>hi</p>").startsWith("<html><head><meta")).toBe(true);
  });

  it("removes every way to load or run something", () => {
    const out = parse(
      previewDocument(`<head>
        <meta http-equiv="refresh" content="0; url=http://x/r"><base href="http://x/">
        <link rel="stylesheet" href="http://x/s.css"><script src="http://x/a.js"></script>
        <style>@import url("http://x/i.css"); p { color: red }</style>
      </head><body onload="go()">
        <img src="http://x/i.png" srcset="http://x/2x.png 2x"><img src="data:image/png;base64,AA==">
        <iframe src="http://x/f"></iframe><video poster="http://x/p.png"></video><object data="http://x/o"></object>
        <svg><image href="http://x/svg.png"></image></svg>
        <a href="http://x/link" ping="http://x/ping">link</a><form action="http://x/f"><button formaction="http://x/b">b</button></form>
      </body>`),
    );
    expect(out.querySelectorAll("script, link, base, meta[http-equiv='refresh']")).toHaveLength(0);
    expect(out.querySelector("meta[http-equiv]")!.getAttribute("http-equiv")).toBe("Content-Security-Policy");
    const html = out.documentElement.outerHTML;
    expect(html).not.toContain("http://x/");
    expect(html).toContain("data:image/png;base64,AA==");
    expect(out.body.getAttribute("onload")).toBeNull();
    expect(out.querySelector("a")!.getAttribute("href")).toBe("#");
    expect(out.querySelector("style:not(:first-of-type)")!.textContent).toContain("p { color: red }");
  });
});
