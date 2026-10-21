import { Plus, X } from "lucide-react";
import { useState } from "react";

import type { Scheme } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatStatusRanges, parseStatusRanges, type KvMatch, type Matcher } from "@/lib/rules";
import { cn } from "@/lib/utils";

import { Checkbox, Choice, Field, NumberInput, PatternInput } from "./fields";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] as const;
const SCHEMES = [
  { value: "any", label: "Any" },
  { value: "http", label: "http" },
  { value: "https", label: "https" },
] as const;

/**
 * What a rule matches: request conditions (all must hold) plus an optional
 * response matcher that gates only the response-phase actions.
 */
export function MatcherBuilder({ matcher, onChange }: { matcher: Matcher; onChange: (m: Matcher) => void }) {
  const set = (patch: Partial<Matcher>) => onChange({ ...matcher, ...patch });
  const toggleMethod = (m: string) =>
    set({ methods: matcher.methods.includes(m) ? matcher.methods.filter((x) => x !== m) : [...matcher.methods, m] });
  return (
    <div className="flex flex-col gap-1.5" data-testid="matcher-builder">
      <Field label="Methods">
        <div className="flex flex-wrap gap-1">
          {METHODS.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => toggleMethod(m)}
              className={cn(
                "h-5 rounded-sm border px-1.5 font-mono text-2xs",
                matcher.methods.includes(m) ? "border-primary bg-primary/15 text-foreground" : "text-muted-foreground",
              )}
            >
              {m}
            </button>
          ))}
          <span className="text-muted-foreground self-center text-2xs">{matcher.methods.length ? "" : "any method"}</span>
        </div>
      </Field>
      <Field label="Scheme · port">
        <Choice
          value={matcher.scheme ?? "any"}
          options={SCHEMES}
          onChange={(v) => set({ scheme: v === "any" ? null : (v as Scheme) })}
          className="w-20"
        />
        <NumberInput value={matcher.port} onChange={(port) => set({ port })} min={1} max={65535} placeholder="any port" />
      </Field>
      <Field label="Host">
        <PatternInput value={matcher.host} onChange={(host) => set({ host })} optional placeholder="any host, e.g. *.example.com" testId="matcher-host" />
      </Field>
      <Field label="Path">
        <PatternInput value={matcher.path} onChange={(path) => set({ path })} optional placeholder="any path, e.g. /api/*" testId="matcher-path" />
      </Field>
      <KvRows label="Query" rows={matcher.query} onChange={(query) => set({ query })} />
      <KvRows label="Headers" rows={matcher.headers} onChange={(headers) => set({ headers })} />
      <Field label="Client app">
        <PatternInput value={matcher.client_app} onChange={(client_app) => set({ client_app })} optional placeholder="any app, e.g. Safari" />
      </Field>
      <ResponseMatcherRow matcher={matcher} onChange={onChange} />
    </div>
  );
}

function KvRows({ label, rows, onChange }: { label: string; rows: KvMatch[]; onChange: (rows: KvMatch[]) => void }) {
  const update = (i: number, patch: Partial<KvMatch>) => onChange(rows.map((r, j) => (i === j ? { ...r, ...patch } : r)));
  return (
    <>
      {rows.map((row, i) => (
        <Field key={i} label={i === 0 ? label : ""}>
          <Input
            value={row.name}
            onChange={(e) => update(i, { name: e.target.value })}
            placeholder="name"
            className="h-6 w-32 shrink-0 font-mono text-xs"
          />
          <PatternInput value={row.value} onChange={(value) => update(i, { value })} optional placeholder="present" />
          <Checkbox checked={row.negate} onChange={(negate) => update(i, { negate })} label="not" />
          <Button variant="ghost" size="icon-xs" aria-label="Remove" onClick={() => onChange(rows.filter((_, j) => j !== i))}>
            <X />
          </Button>
        </Field>
      ))}
      <Field label={rows.length ? "" : label}>
        <Button
          variant="ghost"
          size="xs"
          className="text-muted-foreground"
          onClick={() => onChange([...rows, { name: "", value: null, negate: false }])}
        >
          <Plus />
          {label === "Query" ? "Query parameter" : "Header"}
        </Button>
      </Field>
    </>
  );
}

function ResponseMatcherRow({ matcher, onChange }: { matcher: Matcher; onChange: (m: Matcher) => void }) {
  const response = matcher.response;
  const [statusText, setStatusText] = useState(() => formatStatusRanges(response?.status ?? []));
  const statusValid = parseStatusRanges(statusText) !== null;
  if (!response) {
    return (
      <Field label="Response">
        <Button
          variant="ghost"
          size="xs"
          className="text-muted-foreground"
          onClick={() => onChange({ ...matcher, response: { status: [], content_type: null } })}
        >
          <Plus />
          Only for some responses
        </Button>
      </Field>
    );
  }
  return (
    <>
      <Field label="Response status">
        <Input
          value={statusText}
          aria-invalid={!statusValid}
          placeholder="any, e.g. 2xx, 404, 500-599"
          onChange={(e) => {
            setStatusText(e.target.value);
            const status = parseStatusRanges(e.target.value);
            if (status) onChange({ ...matcher, response: { ...response, status } });
          }}
          className="h-6 font-mono text-xs"
        />
        <Button variant="ghost" size="icon-xs" aria-label="Remove response matcher" onClick={() => onChange({ ...matcher, response: null })}>
          <X />
        </Button>
      </Field>
      <Field label="Content-Type">
        <PatternInput
          value={response.content_type}
          onChange={(content_type) => onChange({ ...matcher, response: { ...response, content_type } })}
          optional
          placeholder="any, e.g. application/json*"
        />
      </Field>
    </>
  );
}
