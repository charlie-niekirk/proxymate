import { CircleCheck, CircleX, FlaskConical, LoaderCircle } from "lucide-react";

import type { RequestHead, ResponseHead, RuleTestResult_Serialize } from "@/bindings";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { headersBlock } from "@/lib/copyFormats";
import { displayHeaderName } from "@/lib/http";

/**
 * "Test against selected flow": the rule's dry run, computed in Rust with
 * the proxy's own evaluator. Nothing is sent.
 */
export function RuleTestPanel({
  flowId,
  result,
  error,
  busy,
  onTest,
}: {
  flowId: number | null;
  result: RuleTestResult_Serialize | null;
  error: string | null;
  busy: boolean;
  onTest: () => void;
}) {
  return (
    <div className="flex flex-col gap-2" data-testid="rule-test">
      <div className="flex items-center gap-2">
        <Button variant="outline" size="xs" disabled={flowId === null || busy} onClick={onTest} data-testid="test-rule">
          {busy ? <LoaderCircle className="animate-spin" /> : <FlaskConical />}
          Test against selected flow
        </Button>
        <span className="text-muted-foreground text-xs">
          {flowId === null ? "Select a flow in the table first" : `Flow #${flowId + 1}`}
        </span>
      </div>
      {error && <div className="text-xs text-[var(--s-failed)]">{error}</div>}
      {result && (
        <div className="flex flex-col gap-2 text-xs" data-testid="rule-test-result">
          <div className="flex items-center gap-1.5 font-medium">
            {result.matched ? (
              <CircleCheck className="size-3.5 text-[var(--s-2xx)]" />
            ) : (
              <CircleX className="size-3.5 text-[var(--s-failed)]" />
            )}
            {result.matched ? "The rule matches this flow" : `No match: ${result.mismatch ?? "the matcher doesn't match"}`}
            {result.matched && result.response_matched === false && (
              <span className="text-muted-foreground font-normal">· its response matcher doesn&apos;t match</span>
            )}
          </div>
          {result.notes.length > 0 && (
            <CopyMenu items={[{ label: "Copy notes", text: result.notes.join("\n"), what: "notes" }]}>
              <ul className="text-muted-foreground list-disc pl-5" data-selectable>
                {result.notes.map((n) => (
                  <li key={n}>{n}</li>
                ))}
              </ul>
            </CopyMenu>
          )}
          {result.matched && (
            <div className="grid grid-cols-2 gap-2">
              <Message title="Request" head={result.request} body={result.request_body} />
              {result.response ? (
                <Message title="Response" head={result.response} body={result.response_body} />
              ) : (
                <div className="text-muted-foreground rounded-md border p-2">No response</div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Message({ title, head, body }: { title: string; head: RequestHead | ResponseHead; body: string | null }) {
  const start = "method" in head ? `${head.method} ${head.url}` : `${head.status} ${head.reason}`;
  const headers = headersBlock(head.headers.map((h) => [displayHeaderName(h.name), h.value]));
  const what = title.toLowerCase();
  return (
    // The copies take the whole body; the panel shows its first 4000 characters.
    <CopyMenu
      items={[
        { label: `Copy ${what}`, text: `${start}\n${headers}${body ? `\n\n${body}` : ""}`, what },
        { label: "Copy headers", text: headers, what: "headers" },
        body !== null && { label: "Copy body", text: body, what: "body" },
      ]}
    >
      <div className="flex min-w-0 flex-col rounded-md border" data-selectable>
        <div className="bg-muted/40 border-b px-2 py-0.5 font-semibold">{title}</div>
        <pre className="max-h-56 overflow-auto px-2 py-1 font-mono text-2xs leading-4 break-all whitespace-pre-wrap">
          {start}
          {"\n"}
          {head.headers.map((h) => `${displayHeaderName(h.name)}: ${h.value}`).join("\n")}
          {body ? `\n\n${body.length > 4000 ? `${body.slice(0, 4000)}…` : body}` : ""}
        </pre>
      </div>
    </CopyMenu>
  );
}
