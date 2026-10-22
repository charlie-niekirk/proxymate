import { useQuery } from "@tanstack/react-query";
import { WandSparkles, WrapText } from "lucide-react";
import { useMemo, useState } from "react";

import { Toggle } from "@/components/ui/toggle";
import { formatMarkup } from "@/lib/body/markup";
import { PREVIEW_CSP, previewDocument } from "@/lib/body/htmlPreview";

import { CodeView } from "./CodeView";
import { Bar, Centered, Note } from "./parts";
import { useText } from "./util";
import { beautify, objectId } from "./workers/client";

/** Bodies above this aren't re-indented: the formatter is a single pass, but 2 MB of tags is plenty. */
const FORMAT_MAX = 2 * 1024 * 1024;

/** Plain text with a wrap toggle. */
export function TextView({ bytes }: { bytes: Uint8Array }) {
  const text = useText(bytes);
  const [wrap, setWrap] = useState(true);
  if (text === null) return <Centered>Binary body: switch to Hex to see its bytes.</Centered>;
  return (
    <div className="flex h-full flex-col">
      <Bar>
        <Toggle size="sm" pressed={wrap} onPressedChange={setWrap} aria-label="Wrap lines">
          <WrapText /> Wrap
        </Toggle>
      </Bar>
      <div className="min-h-0 flex-1">
        <CodeView text={text} language="text" wrap={wrap} />
      </div>
    </div>
  );
}

/** XML or HTML, re-indented and highlighted. */
export function MarkupView({ bytes, html, highlight }: { bytes: Uint8Array; html: boolean; highlight: boolean }) {
  const text = useText(bytes);
  const pretty = useMemo(() => (text !== null && text.length <= FORMAT_MAX ? formatMarkup(text, { html }) : text), [text, html]);
  if (pretty === null) return <Centered>Binary body: switch to Hex to see its bytes.</Centered>;
  return <CodeView text={pretty} language={html ? "html" : "xml"} highlight={highlight} />;
}

/**
 * The rendered page in an iframe with `sandbox` (no scripts, forms, popups
 * or top navigation), stripped of remote URLs and under a CSP that blocks
 * every network load (see lib/body/htmlPreview.ts).
 */
export function HtmlPreview({ bytes }: { bytes: Uint8Array }) {
  const text = useText(bytes);
  const doc = useMemo(() => (text === null ? null : previewDocument(text)), [text]);
  if (doc === null) return <Centered>Binary body: nothing to preview.</Centered>;
  return (
    <div className="flex h-full flex-col">
      <Note>Scripts, forms and network loads are blocked in this preview.</Note>
      <iframe
        title="HTML preview"
        sandbox=""
        srcDoc={doc}
        referrerPolicy="no-referrer"
        // CSP Embedded Enforcement (Chromium, so WebView2); WebKit ignores it.
        {...{ csp: PREVIEW_CSP }}
        className="min-h-0 w-full flex-1 border-0 bg-paper"
        data-testid="html-preview"
      />
    </div>
  );
}

/** JavaScript or CSS, highlighted, with an on-demand prettier pass in a worker. */
export function ScriptView({ bytes, lang, highlight }: { bytes: Uint8Array; lang: "javascript" | "css"; highlight: boolean }) {
  const text = useText(bytes);
  // Minified code reads better beautified; start that way when it's cheap.
  const minified = useMemo(() => text !== null && text.length < 512 * 1024 && averageLineLength(text) > 200, [text]);
  const [pretty, setPretty] = useState(minified);
  const beautified = useQuery({
    queryKey: ["beautify", lang, objectId(bytes)],
    enabled: pretty && text !== null,
    queryFn: () => beautify(text!, lang),
    gcTime: 0,
  });
  if (text === null) return <Centered>Binary body: switch to Hex to see its bytes.</Centered>;
  const shown = pretty && beautified.data ? beautified.data : text;
  return (
    <div className="flex h-full flex-col">
      <Bar>
        <Toggle size="sm" pressed={pretty} onPressedChange={setPretty} aria-label="Beautify" data-testid="beautify">
          <WandSparkles /> Beautify
        </Toggle>
        {pretty && beautified.isPending && <span className="text-muted-foreground">Formatting…</span>}
        {pretty && beautified.isError && <span className="text-[var(--s-failed)]">Couldn&apos;t beautify: {beautified.error.message}</span>}
      </Bar>
      <div className="min-h-0 flex-1">
        <CodeView text={shown} language={lang} highlight={highlight} wrap={!pretty} />
      </div>
    </div>
  );
}

function averageLineLength(text: string): number {
  const lines = text.split("\n").length;
  return text.length / lines;
}
