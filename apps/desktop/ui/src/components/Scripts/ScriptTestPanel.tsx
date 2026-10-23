import { FlaskConical, LoaderCircle, TriangleAlert } from "lucide-react";
import { useState } from "react";

import type { BodyPart, RequestHead, ResponseHead, ScriptKind, ScriptTestResult_Serialize } from "@/bindings";
import { ScriptViewBody } from "@/components/BodyViewer/ScriptViewBody";
import { Choice } from "@/components/Rules/fields";
import { Button } from "@/components/ui/button";
import { displayHeaderName } from "@/lib/http";
import { cn } from "@/lib/utils";

import { ConsoleLine } from "./ScriptConsole";

/**
 * A script's dry run against the selected flow (computed in Rust, in a
 * worker of its own): before and after for rewrite scripts, the answer for
 * mock scripts, the views for view scripts. Nothing is sent or saved.
 */
export function ScriptTestPanel({
  kind,
  flowId,
  result,
  error,
  busy,
  onTest,
}: {
  kind: ScriptKind;
  flowId: number | null;
  result: ScriptTestResult_Serialize | null;
  error: string | null;
  busy: boolean;
  onTest: (part?: BodyPart) => void;
}) {
  const [part, setPart] = useState<BodyPart>("response");
  return (
    <div className="flex flex-col gap-2 text-xs" data-testid="script-test">
      <div className="flex items-center gap-2">
        <Button variant="outline" size="xs" disabled={flowId === null || busy} onClick={() => onTest(part)} data-testid="test-script">
          {busy ? <LoaderCircle className="animate-spin" /> : <FlaskConical />}
          Test against selected flow
        </Button>
        {kind === "view" && (
          <Choice
            value={part}
            options={[
              { value: "response", label: "Response body" },
              { value: "request", label: "Request body" },
            ]}
            onChange={setPart}
            className="h-6 w-36 text-xs"
          />
        )}
        <span className="text-muted-foreground">
          {flowId === null ? "Select a flow in the table first" : `Flow #${flowId + 1}`}
        </span>
      </div>
      {error && <div className="text-[var(--s-failed)]">{error}</div>}
      {result && (
        <div className="flex flex-col gap-2" data-testid="script-test-result">
          {result.error && (
            <pre className="flex gap-1.5 rounded-md border border-[var(--s-failed)]/40 p-2 font-mono text-2xs whitespace-pre-wrap text-[var(--s-failed)]" data-testid="script-test-error">
              <TriangleAlert className="mt-0.5 size-3 shrink-0" />
              {result.error}
            </pre>
          )}
          {result.notes.length > 0 && (
            <ul className="text-muted-foreground list-disc pl-5">
              {result.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          )}
          {result.kind === "rewrite" && (
            <>
              <BeforeAfter title="Request" before={result.request_before} beforeBody={result.request_body_before} after={result.request} afterBody={result.request_body} />
              {result.response_before && (
                <BeforeAfter
                  title="Response"
                  before={result.response_before}
                  beforeBody={result.response_body_before}
                  after={result.response}
                  afterBody={result.response_body}
                />
              )}
            </>
          )}
          {result.kind === "mock" && result.response && (
            <Message title="Mocked response" head={result.response} body={result.response_body} />
          )}
          {result.kind === "view" &&
            result.views.map((view, i) => (
              <div key={i} className="flex flex-col rounded-md border" data-testid="script-test-view">
                <div className="bg-muted/40 border-b px-2 py-0.5 font-semibold">
                  {view.title} <span className="text-muted-foreground font-normal">· {view.kind}</span>
                </div>
                <div className="h-56">
                  <ScriptViewBody
                    option={{ value: `test:${i}`, label: view.title, scriptId: "", scriptName: "", view }}
                    bodyId={`test-${i}`}
                  />
                </div>
              </div>
            ))}
          {result.console.length > 0 && (
            <div className="rounded-md border" data-testid="script-test-console">
              <div className="bg-muted/40 border-b px-2 py-0.5 font-semibold">Console</div>
              {result.console.map((entry, i) => (
                <ConsoleLine key={i} entry={entry} />
              ))}
            </div>
          )}
          <div className="text-muted-foreground">Ran in {(result.duration_ms ?? 0).toFixed(1)} ms</div>
        </div>
      )}
    </div>
  );
}

function headText(head: RequestHead | ResponseHead): string[] {
  const start = "method" in head ? `${head.method} ${head.url}` : `${head.status} ${head.reason}`;
  return [start, ...head.headers.map((h) => `${displayHeaderName(h.name)}: ${h.value}`)];
}

/** Before and after side by side; changed lines are marked. */
function BeforeAfter({
  title,
  before,
  beforeBody,
  after,
  afterBody,
}: {
  title: string;
  before: RequestHead | ResponseHead | null;
  beforeBody: string | null;
  after: RequestHead | ResponseHead | null;
  afterBody: string | null;
}) {
  if (!before || !after) return null;
  const a = [...headText(before), "", ...(beforeBody ?? "").split("\n")];
  const b = [...headText(after), "", ...(afterBody ?? "").split("\n")];
  const inA = new Set(a);
  const inB = new Set(b);
  return (
    <div className="grid grid-cols-2 gap-2" data-testid={`script-diff-${title.toLowerCase()}`}>
      <Lines title={`${title} before`} lines={a} changed={(l) => !inB.has(l)} tone="removed" />
      <Lines title={`${title} after`} lines={b} changed={(l) => !inA.has(l)} tone="added" />
    </div>
  );
}

function Lines({
  title,
  lines,
  changed,
  tone,
}: {
  title: string;
  lines: string[];
  changed: (line: string) => boolean;
  tone: "added" | "removed";
}) {
  return (
    <div className="flex min-w-0 flex-col rounded-md border" data-selectable>
      <div className="bg-muted/40 border-b px-2 py-0.5 font-semibold">{title}</div>
      <pre className="max-h-64 overflow-auto px-2 py-1 font-mono text-2xs leading-4 break-all whitespace-pre-wrap">
        {lines.slice(0, 400).map((line, i) => (
          <div
            key={i}
            className={cn(changed(line) && line !== "" && (tone === "added" ? "bg-[var(--s-2xx-bg)]" : "bg-[var(--s-5xx-bg)]"))}
            data-changed={changed(line) && line !== "" ? tone : undefined}
          >
            {line || " "}
          </div>
        ))}
      </pre>
    </div>
  );
}

function Message({ title, head, body }: { title: string; head: RequestHead | ResponseHead; body: string | null }) {
  return (
    <div className="flex min-w-0 flex-col rounded-md border" data-selectable>
      <div className="bg-muted/40 border-b px-2 py-0.5 font-semibold">{title}</div>
      <pre className="max-h-64 overflow-auto px-2 py-1 font-mono text-2xs leading-4 break-all whitespace-pre-wrap">
        {headText(head).join("\n")}
        {body ? `\n\n${body.length > 8000 ? `${body.slice(0, 8000)}…` : body}` : ""}
      </pre>
    </div>
  );
}
