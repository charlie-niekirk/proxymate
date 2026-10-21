import { useMemo, useState } from "react";

import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { parseGraphQlBody, type GraphQlOperation } from "@/lib/body/graphql";

import { CodeView } from "./CodeView";
import { Bar, Note } from "./parts";
import { TextView } from "./TextViews";
import { useText } from "./util";

/** A GraphQL request: the query next to its variables, per operation of a batch. */
export function GraphQlView({ bytes, contentType }: { bytes: Uint8Array; contentType: string | null }) {
  const text = useText(bytes);
  const ops = useMemo(() => (text === null ? null : parseGraphQlBody(text, contentType)), [text, contentType]);
  if (!ops) {
    return (
      <div className="flex h-full flex-col">
        <Note tone="warn">Not a GraphQL request body. Showing the text.</Note>
        <div className="min-h-0 flex-1">
          <TextView bytes={bytes} />
        </div>
      </div>
    );
  }
  return <Operations ops={ops} />;
}

export function Operations({ ops }: { ops: GraphQlOperation[] }) {
  const [index, setIndex] = useState(0);
  const op = ops[Math.min(index, ops.length - 1)]!;
  const variables = op.variables === null ? null : JSON.stringify(op.variables, null, 2);
  const extensions = op.extensions === null ? null : JSON.stringify(op.extensions, null, 2);
  return (
    <div className="flex h-full flex-col" data-testid="graphql-view">
      <Bar>
        {ops.length > 1 && (
          <ToggleGroup type="single" size="sm" value={String(index)} onValueChange={(v) => v && setIndex(Number(v))}>
            {ops.map((o, i) => (
              <ToggleGroupItem key={i} value={String(i)}>
                {o.operationName ?? `#${i + 1}`}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        )}
        {op.type && <span className="font-mono text-[var(--syntax-keyword)]">{op.type}</span>}
        <span className="font-mono font-semibold">{op.operationName ?? "anonymous"}</span>
        {ops.length > 1 && <span className="text-muted-foreground ml-auto">batch of {ops.length}</span>}
      </Bar>
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        <ResizablePanel defaultSize="58" minSize={120}>
          <Pane title="Query">
            {op.query ? <CodeView text={op.query} language="graphql" /> : <Empty text="Persisted query: no query text was sent" />}
          </Pane>
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize="42" minSize={120}>
          <div className="flex h-full flex-col">
            <Pane title="Variables">
              {variables ? <CodeView text={variables} language="json" /> : <Empty text="No variables" />}
            </Pane>
            {extensions && (
              <Pane title="Extensions">
                <CodeView text={extensions} language="json" />
              </Pane>
            )}
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );
}

function Pane({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="text-muted-foreground flex h-5 shrink-0 items-center border-b bg-[var(--pane-header)] px-2 text-[10.5px] font-medium tracking-wide uppercase">
        {title}
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return <div className="text-muted-foreground p-3 text-xs">{text}</div>;
}
