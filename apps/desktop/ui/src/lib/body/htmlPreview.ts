/**
 * Turns an HTML body into a document that is safe to render in the preview
 * iframe. Three layers, because each engine leaks through a different one:
 *   1. The iframe is sandboxed without `allow-scripts`: no script runs.
 *   2. Every remote URL is removed from the markup before it is rendered,
 *      so nothing (not even Chromium's preload scanner) can fetch it.
 *   3. A CSP blocks any load that slips through (CSS `url()`, …); the iframe
 *      also carries it as `csp` for engines that support that attribute.
 */

export const PREVIEW_CSP =
  "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; media-src data:; form-action 'none'";

/** Attributes that make an element fetch (or, on links, navigate to) a URL. */
const URL_ATTRS = ["src", "srcset", "href", "xlink:href", "poster", "data", "background", "action", "formaction", "ping", "lowsrc", "imagesrcset", "manifest", "codebase", "archive", "longdesc", "cite"];

/** Elements that only load or run things. */
const DROP = "script, noscript, base, link, meta[http-equiv]:not([http-equiv='content-type' i]), portal, applet";

const isInline = (url: string) => /^\s*(data:|#|$)/i.test(url);

export function previewDocument(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  for (const el of doc.querySelectorAll(DROP)) el.remove();
  for (const el of doc.querySelectorAll("*")) {
    for (const attr of [...el.attributes]) {
      const name = attr.name.toLowerCase();
      if (name.startsWith("on")) el.removeAttribute(attr.name);
      else if (URL_ATTRS.includes(name) && !isInline(attr.value)) {
        // Links keep their place (and look) but go nowhere.
        if (name === "href" && el.localName === "a") el.setAttribute("href", "#");
        else el.removeAttribute(attr.name);
      }
    }
  }
  // Preload scanners read @import from <style> text too.
  for (const style of doc.querySelectorAll("style")) {
    style.textContent = (style.textContent ?? "").replace(/@import\b[^;]*;?/gi, "/* @import removed */");
  }
  const meta = doc.createElement("meta");
  meta.httpEquiv = "Content-Security-Policy";
  meta.content = PREVIEW_CSP;
  const guard = doc.createElement("style");
  guard.textContent = "a, area { pointer-events: none !important; }";
  doc.head.prepend(meta, guard);
  const doctype = doc.doctype ? `<!DOCTYPE ${doc.doctype.name}>` : "";
  return doctype + doc.documentElement.outerHTML;
}
