//! The Rewrite dialog's body editor: a JSON or GraphQL tree, form rows, or
//! text (replace it, or find and replace).

import { languageFor } from "@/lib/compose";
import { Centered } from "@/components/BodyViewer/parts";
import { CodeEditor } from "@/components/Compose/CodeEditor";
import { JsonEditor } from "@/components/JsonEditor/JsonEditor";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { MANY_ACTIONS, type BodySource } from "@/lib/rewrite/session";

import { KvEditor } from "./KvEditor";
import { Scroll, Segmented } from "./parts";
import type { BodyState } from "./useBody";

export function WholeToggle({ body }: { body: BodyState }) {
  return (
    <label className="flex items-center gap-1.5 text-[11px]" title="Send the edited body instead of patching only what changed">
      <Switch size="sm" checked={body.whole} onCheckedChange={body.setWhole} data-testid="rewrite-whole-body" />
      Replace whole body
    </label>
  );
}

export function BodyPane({
  source,
  body,
  contentType,
  replaced,
  many,
}: {
  source: BodySource;
  body: BodyState;
  contentType: string | null;
  replaced: ReadonlySet<number>;
  many: boolean;
}) {
  const hint = many && !body.whole && (
    <div className="border-b bg-[var(--s-4xx-bg)] px-2 py-1 text-xs" data-testid="rewrite-many">
      Over {MANY_ACTIONS} changes: “Replace whole body” sends one body instead.
    </div>
  );
  switch (source.kind) {
    case "none":
      return <Centered>{source.note ?? "No body."}</Centered>;
    case "json":
    case "graphql":
      return (
        <div className="flex h-full flex-col">
          {source.kind === "graphql" && (
            <div className="text-muted-foreground border-b px-2 py-1 text-xs" data-testid="rewrite-graphql">
              GraphQL variables{source.via === "query" ? " (the variables query parameter)" : ""}
            </div>
          )}
          {hint}
          <div className="min-h-0 flex-1">
            <JsonEditor
              doc={body.tree}
              original={source.kind === "json" ? source.doc : source.vars}
              replaced={replaced}
              toolbar={source.kind === "json" || source.via === "body" ? <WholeToggle body={body} /> : null}
              testId="rewrite-tree"
            />
          </div>
        </div>
      );
    case "form":
      return (
        <div className="flex h-full flex-col">
          <div className="flex h-7 shrink-0 items-center border-b px-2">
            <span className="text-muted-foreground text-xs">Form fields</span>
            <div className="ml-auto">
              <WholeToggle body={body} />
            </div>
          </div>
          <Scroll>
            <KvEditor rows={body.rows} original={source.rows} onChange={body.setRows} repeatedReadOnly addLabel="Field" testId="rewrite-form" />
          </Scroll>
        </div>
      );
    case "text":
      return (
        <div className="flex h-full flex-col">
          {source.note && <div className="text-muted-foreground border-b px-2 py-1 text-xs">{source.note}</div>}
          <div className="flex h-7 shrink-0 items-center border-b px-2">
            <Segmented
              value={body.mode}
              onChange={body.setMode}
              options={[
                { value: "find", label: "Find & replace", testId: "rewrite-text-find" },
                { value: "replace", label: "Replace whole body", testId: "rewrite-text-replace" },
              ]}
            />
          </div>
          {body.mode === "replace" ? (
            <div className="min-h-0 flex-1">
              <CodeEditor value={body.text} language={languageFor(contentType)} onChange={body.setText} testId="rewrite-text" />
            </div>
          ) : (
            <Scroll>
              <FindReplace pairs={body.pairs} onChange={body.setPairs} />
            </Scroll>
          )}
        </div>
      );
  }
}

export function FindReplace({ pairs, onChange }: { pairs: { find: string; replace: string }[]; onChange: (p: { find: string; replace: string }[]) => void }) {
  const rows = [...pairs, { find: "", replace: "" }];
  const update = (i: number, patch: Partial<{ find: string; replace: string }>) => {
    const next = rows.map((p, j) => (i === j ? { ...p, ...patch } : p)).filter((p) => p.find !== "" || p.replace !== "");
    onChange(next);
  };
  return (
    <div className="flex flex-col gap-1 p-2 text-xs" data-testid="rewrite-find-replace">
      <span className="text-muted-foreground">Each text found in the body is replaced, every time it occurs.</span>
      {rows.map((p, i) => (
        <div key={i} className="flex items-center gap-1">
          <Input value={p.find} placeholder="find" onChange={(e) => update(i, { find: e.target.value })} className="h-6 font-mono text-xs" />
          <span className="text-muted-foreground">→</span>
          <Input value={p.replace} placeholder="replace with" onChange={(e) => update(i, { replace: e.target.value })} className="h-6 font-mono text-xs" />
        </div>
      ))}
    </div>
  );
}
