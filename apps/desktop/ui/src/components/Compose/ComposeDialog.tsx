import { Binary, Plus, Send, X } from "lucide-react";
import { useEffect, useState } from "react";

import type { Header, RequestSpec_Serialize } from "@/bindings";
import { CodeEditor } from "@/components/Compose/CodeEditor";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { bridge } from "@/dev/bridge";
import { sendComposed } from "@/lib/actions";
import {
  CONTENT_TYPES,
  METHODS,
  contentTypeOf,
  draftOf,
  languageFor,
  newDraft,
  specOf,
  withContentType,
  type ComposeDraft,
} from "@/lib/compose";
import { formatSize } from "@/lib/flow";
import { useUi } from "@/store/ui";

/** The value of the content-type select for a header value not in the list. */
const CUSTOM = "__custom";
const NONE = "__none";

/**
 * Compose: a new request, or a recorded one to edit and send again (Edit &
 * Replay). Sending records it as a new live flow, marked as replayed, and
 * selects it.
 */
export function ComposeDialog() {
  const { open, spec, seq } = useUi((s) => s.compose);
  const close = useUi((s) => s.closeCompose);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="flex h-[78vh] flex-col gap-0 p-0 sm:max-w-[920px]"
        onOpenAutoFocus={(e) => e.preventDefault()}
        data-testid="compose"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <Send className="size-4" />
            {spec ? "Edit & Replay" : "Compose"}
          </DialogTitle>
          <DialogDescription>
            Sent from Proxymate and recorded as a new flow. Host and Content-Length are set from the URL and the body.
          </DialogDescription>
        </DialogHeader>
        <ComposeForm key={seq} spec={spec} onSent={close} />
      </DialogContent>
    </Dialog>
  );
}

function ComposeForm({ spec, onSent }: { spec: RequestSpec_Serialize | null; onSent: () => void }) {
  const [draft, setDraft] = useState<ComposeDraft>(() => (spec ? draftOf(spec) : newDraft()));
  const [applyRules, setApplyRules] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const update = (patch: Partial<ComposeDraft>) => setDraft((d) => ({ ...d, ...patch }));
  const contentType = contentTypeOf(draft.headers);
  const known = CONTENT_TYPES.find((t) => t.value === (contentType?.split(";")[0]?.trim().toLowerCase() ?? null));
  const typeValue = known ? (known.value ?? NONE) : CUSTOM;

  const send = async () => {
    setError(null);
    setSending(true);
    try {
      await sendComposed(specOf(draft), applyRules);
      onSent();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  };

  // Lets dev/remote.ts press Send in the real app.
  useEffect(() => {
    bridge.sendCompose = send;
    return () => {
      bridge.sendCompose = undefined;
    };
  });

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      onKeyDown={(e) => {
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void send();
      }}
    >
      <div className="flex items-center gap-1.5 border-b px-3 py-2">
        <Input
          value={draft.method}
          onChange={(e) => update({ method: e.target.value.toUpperCase() })}
          list="pm-methods"
          className="h-7 w-24 font-mono text-xs"
          aria-label="Method"
          data-testid="compose-method"
        />
        <datalist id="pm-methods">
          {METHODS.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
        <Input
          value={draft.url}
          onChange={(e) => update({ url: e.target.value })}
          className="h-7 font-mono text-xs"
          aria-label="URL"
          placeholder="https://api.example.com/path?query"
          data-testid="compose-url"
        />
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="flex w-[46%] min-w-0 flex-col overflow-y-auto border-r p-3">
          <HeaderTable rows={draft.headers} onChange={(headers) => update({ headers })} />
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-8 shrink-0 items-center gap-2 border-b px-2 text-xs">
            <span className="text-muted-foreground font-semibold">Body</span>
            <Select
              value={typeValue}
              onValueChange={(v) => {
                if (v === CUSTOM) return;
                update({ headers: withContentType(draft.headers, v === NONE ? null : v) });
              }}
            >
              <SelectTrigger className="h-5 min-w-28 px-1.5 text-[11px]" aria-label="Content type" data-testid="compose-type">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CONTENT_TYPES.map((t) => (
                  <SelectItem key={t.label} value={t.value ?? NONE} className="text-xs">
                    {t.label}
                  </SelectItem>
                ))}
                {typeValue === CUSTOM && (
                  <SelectItem value={CUSTOM} className="text-xs">
                    {contentType}
                  </SelectItem>
                )}
              </SelectContent>
            </Select>
          </div>
          {draft.bodyBase64 ? (
            <BinaryBody size={Math.floor((draft.body.length * 3) / 4)} onClear={() => update({ body: "", bodyBase64: false })} />
          ) : (
            <CodeEditor
              value={draft.body}
              language={languageFor(contentType)}
              onChange={(body) => update({ body })}
              className="min-h-0 flex-1 overflow-hidden"
            />
          )}
        </div>
      </div>
      <div className="flex items-center gap-3 border-t px-3 py-2">
        <label className="text-muted-foreground flex items-center gap-1.5 text-xs">
          <Switch size="sm" checked={applyRules} onCheckedChange={setApplyRules} data-testid="compose-rules" />
          Apply rules
        </label>
        <span className="min-w-0 flex-1 truncate text-xs text-[var(--s-failed)]" data-testid="compose-error">
          {error}
        </span>
        <span className="text-muted-foreground text-2xs">⌘↩</span>
        <Button size="sm" onClick={() => void send()} disabled={sending} data-testid="compose-send">
          <Send />
          Send
        </Button>
      </div>
    </div>
  );
}

function BinaryBody({ size, onClear }: { size: number; onClear: () => void }) {
  return (
    <div className="text-muted-foreground flex flex-1 flex-col items-center justify-center gap-2 text-xs" data-testid="compose-binary">
      <Binary className="size-5" />
      <span>Binary body · {formatSize(size)} · sent as recorded</span>
      <Button variant="outline" size="xs" onClick={onClear}>
        Replace with text
      </Button>
    </div>
  );
}

function HeaderTable({ rows, onChange }: { rows: Header[]; onChange: (rows: Header[]) => void }) {
  const update = (i: number, patch: Partial<Header>) => onChange(rows.map((r, j) => (i === j ? { ...r, ...patch } : r)));
  return (
    <div className="flex flex-col gap-0.5 text-xs" data-testid="compose-headers">
      <span className="text-muted-foreground mb-1 font-semibold">Headers</span>
      {rows.map((h, i) => (
        <div key={i} className="flex items-center gap-1">
          <Input
            value={h.name}
            onChange={(e) => update(i, { name: e.target.value })}
            placeholder="name"
            className="h-6 w-36 shrink-0 font-mono text-xs"
          />
          <Input
            value={h.value}
            onChange={(e) => update(i, { value: e.target.value })}
            placeholder="value"
            className="h-6 min-w-0 font-mono text-xs"
          />
          <Button variant="ghost" size="icon-xs" aria-label="Remove header" onClick={() => onChange(rows.filter((_, j) => j !== i))}>
            <X />
          </Button>
        </div>
      ))}
      <Button
        variant="ghost"
        size="xs"
        className="text-muted-foreground self-start"
        onClick={() => onChange([...rows, { name: "", value: "" }])}
      >
        <Plus />
        Header
      </Button>
    </div>
  );
}
