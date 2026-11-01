import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";
import { useState } from "react";

import type { MapLocalSource, Scheme, WsDirection } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  ACTION_TYPES,
  actionLabel,
  WS_OP_LABELS,
  SERVE_MISS_LABELS,
  SERVE_PICK_LABELS,
  defaultAction,
  defaultWsOp,
  jsonSetText,
  moveItem,
  type Action,
  type ActionType,
  type ServeMiss,
  type ServePick,
  type WsOpKind,
} from "@/lib/rules";

import { parseLossless } from "@/lib/body/losslessJson";
import { useMockSources } from "@/hooks/useMocking";
import { useScripts } from "@/hooks/useScripts";
import { useUi } from "@/store/ui";

import { Checkbox, Choice, Field, NumberInput, PatternInput, PhaseChoice, TextArea } from "./fields";

/**
 * A rule's actions, in order. Every matching rule's actions run; see
 * `docs/PLAN.md` ("Rules engine") for how they combine.
 */
export function ActionListEditor({ actions, onChange }: { actions: Action[]; onChange: (actions: Action[]) => void }) {
  const replace = (i: number, action: Action) => onChange(actions.map((a, j) => (i === j ? action : a)));
  return (
    <div className="flex flex-col gap-2" data-testid="action-list">
      {actions.length === 0 && <div className="text-muted-foreground px-1 text-xs">No actions yet: the rule matches but does nothing.</div>}
      {actions.map((action, i) => (
        <div key={i} className="rounded-md border" data-testid="action">
          <div className="bg-muted/40 flex h-6 items-center gap-1 border-b pr-1 pl-2">
            <span className="flex-1 text-xs font-medium">{actionLabel(action.type)}</span>
            <Button variant="ghost" size="icon-xs" aria-label="Move up" disabled={i === 0} onClick={() => onChange(moveItem(actions, i, i - 1))}>
              <ArrowUp />
            </Button>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label="Move down"
              disabled={i === actions.length - 1}
              onClick={() => onChange(moveItem(actions, i, i + 1))}
            >
              <ArrowDown />
            </Button>
            <Button variant="ghost" size="icon-xs" aria-label="Remove action" onClick={() => onChange(actions.filter((_, j) => j !== i))}>
              <X />
            </Button>
          </div>
          <div className="flex flex-col gap-1.5 p-2">
            <ActionForm action={action} onChange={(a) => replace(i, a)} />
          </div>
        </div>
      ))}
      <AddAction onAdd={(type) => onChange([...actions, defaultAction(type)])} />
    </div>
  );
}

function AddAction({ onAdd }: { onAdd: (type: ActionType) => void }) {
  const [open, setOpen] = useState(false);
  const groups = [...new Set(ACTION_TYPES.map((a) => a.group))];
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="xs" className="self-start" data-testid="add-action">
          <Plus />
          Add action
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="max-h-(--radix-popover-content-available-height) w-64 overflow-y-auto p-1 text-xs">
        {groups.map((group) => (
          <div key={group} className="py-0.5">
            <div className="text-muted-foreground px-2 py-0.5 text-2xs font-semibold uppercase">{group}</div>
            {ACTION_TYPES.filter((a) => a.group === group).map((a) => (
              <button
                key={a.type}
                type="button"
                className="hover:bg-accent w-full rounded-sm px-2 py-1 text-left"
                onClick={() => {
                  onAdd(a.type);
                  setOpen(false);
                }}
              >
                {a.label}
              </button>
            ))}
          </div>
        ))}
      </PopoverContent>
    </Popover>
  );
}

const MONO = "h-6 font-mono text-xs";

function ActionForm({ action, onChange }: { action: Action; onChange: (action: Action) => void }) {
  switch (action.type) {
    case "SetHeader":
    case "AddHeader":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="Header">
            <Input className={MONO} value={action.name} placeholder="name" onChange={(e) => onChange({ ...action, name: e.target.value })} />
            <Input className={MONO} value={action.value} placeholder="value" onChange={(e) => onChange({ ...action, value: e.target.value })} />
          </Field>
        </>
      );
    case "RemoveHeader":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="Header">
            <Input className={MONO} value={action.name} placeholder="name" onChange={(e) => onChange({ ...action, name: e.target.value })} />
          </Field>
        </>
      );
    case "ReplaceHeaderValue":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="Header">
            <Input className={MONO} value={action.name} placeholder="name" onChange={(e) => onChange({ ...action, name: e.target.value })} />
          </Field>
          <Field label="Find">
            <PatternInput value={action.pattern} onChange={(p) => p && onChange({ ...action, pattern: p })} />
          </Field>
          <Field label="Replace with">
            <Input className={MONO} value={action.replacement} placeholder="$1 for a regex group" onChange={(e) => onChange({ ...action, replacement: e.target.value })} />
          </Field>
        </>
      );
    case "SetQuery":
      return (
        <Field label="Parameter">
          <Input className={MONO} value={action.name} placeholder="name" onChange={(e) => onChange({ ...action, name: e.target.value })} />
          <Input className={MONO} value={action.value} placeholder="value" onChange={(e) => onChange({ ...action, value: e.target.value })} />
        </Field>
      );
    case "RemoveQuery":
      return (
        <Field label="Parameter">
          <Input className={MONO} value={action.name} placeholder="name" onChange={(e) => onChange({ ...action, name: e.target.value })} />
        </Field>
      );
    case "SetBody":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
            <Input
              className={MONO}
              value={action.content_type ?? ""}
              placeholder="Content-Type (unchanged)"
              onChange={(e) => onChange({ ...action, content_type: e.target.value || null })}
            />
          </Field>
          <TextArea value={action.body} rows={5} onChange={(e) => onChange({ ...action, body: e.target.value })} />
        </>
      );
    case "ReplaceBody":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="Find">
            <PatternInput value={action.pattern} onChange={(p) => p && onChange({ ...action, pattern: p })} />
          </Field>
          <Field label="Replace with">
            <Input className={MONO} value={action.replacement} onChange={(e) => onChange({ ...action, replacement: e.target.value })} />
          </Field>
        </>
      );
    case "JsonSet":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="JSON pointer">
            <Input className={MONO} value={action.pointer} placeholder="/data/items/0/name" onChange={(e) => onChange({ ...action, pointer: e.target.value })} />
          </Field>
          <Field label="Value (JSON)">
            <JsonValueInput value={jsonSetText(action)} onChange={(raw) => onChange({ ...action, value: null, raw })} />
          </Field>
        </>
      );
    case "FormSet":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="Field">
            <Input className={MONO} value={action.name} placeholder="name" onChange={(e) => onChange({ ...action, name: e.target.value })} />
            <Input className={MONO} value={action.value} placeholder="value" onChange={(e) => onChange({ ...action, value: e.target.value })} />
          </Field>
        </>
      );
    case "FormRemove":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="Field">
            <Input className={MONO} value={action.name} placeholder="name" onChange={(e) => onChange({ ...action, name: e.target.value })} />
          </Field>
        </>
      );
    case "JsonRemove":
      return (
        <>
          <Field label="Phase">
            <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          </Field>
          <Field label="JSON pointer">
            <Input className={MONO} value={action.pointer} placeholder="/data/secret" onChange={(e) => onChange({ ...action, pointer: e.target.value })} />
          </Field>
        </>
      );
    case "SetStatus":
      return (
        <Field label="Status">
          <NumberInput value={action.status} min={100} max={999} onChange={(s) => onChange({ ...action, status: s ?? 200 })} />
        </Field>
      );
    case "MapLocal":
      return <MapLocalForm action={action} onChange={onChange} />;
    case "MapRemote":
      return (
        <>
          <Field label="Scheme">
            <Choice
              value={action.scheme ?? "same"}
              options={[
                { value: "same", label: "Same" },
                { value: "http", label: "http" },
                { value: "https", label: "https" },
              ]}
              onChange={(v) => onChange({ ...action, scheme: v === "same" ? null : (v as Scheme) })}
              className="w-20"
            />
          </Field>
          <Field label="Host · port">
            <Input className={MONO} value={action.host ?? ""} placeholder="same host" onChange={(e) => onChange({ ...action, host: e.target.value || null })} />
            <NumberInput value={action.port} min={1} max={65535} placeholder="default" onChange={(port) => onChange({ ...action, port })} />
          </Field>
          <Field label="Path">
            <Input className={MONO} value={action.path ?? ""} placeholder="same path" onChange={(e) => onChange({ ...action, path: e.target.value || null })} />
          </Field>
          <Field label="">
            <Checkbox checked={action.preserve_host} onChange={(preserve_host) => onChange({ ...action, preserve_host })} label="Keep the original Host header" />
          </Field>
        </>
      );
    case "Breakpoint":
      return (
        <Field label="Pause at">
          <Checkbox checked={action.request} onChange={(request) => onChange({ ...action, request })} label="Request" testId="bp-request" />
          <Checkbox checked={action.response} onChange={(response) => onChange({ ...action, response })} label="Response" testId="bp-response" />
        </Field>
      );
    case "Delay":
      return (
        <Field label="Delay">
          <PhaseChoice value={action.phase} onChange={(phase) => onChange({ ...action, phase })} />
          <NumberInput value={action.ms} onChange={(ms) => onChange({ ...action, ms: ms ?? 0 })} />
          <span className="text-muted-foreground text-xs">ms</span>
        </Field>
      );
    case "Throttle":
      return (
        <>
          <Field label="Down · up">
            <NumberInput value={action.download_kbps} onChange={(v) => onChange({ ...action, download_kbps: v ?? 0 })} />
            <NumberInput value={action.upload_kbps} onChange={(v) => onChange({ ...action, upload_kbps: v ?? 0 })} />
            <span className="text-muted-foreground text-xs">kbps (0: unlimited)</span>
          </Field>
          <Field label="Latency">
            <NumberInput value={action.latency_ms} onChange={(v) => onChange({ ...action, latency_ms: v ?? 0 })} />
            <span className="text-muted-foreground text-xs">ms</span>
          </Field>
        </>
      );
    case "Block":
      return (
        <Field label="Block with">
          <Choice
            value={action.mode.kind}
            options={[
              { value: "status", label: "A status" },
              { value: "reset", label: "A connection reset" },
            ]}
            onChange={(kind) => onChange({ ...action, mode: kind === "reset" ? { kind } : { kind, status: 403 } })}
            className="w-40"
          />
          {action.mode.kind === "status" && (
            <NumberInput
              value={action.mode.status}
              min={100}
              max={999}
              onChange={(status) => onChange({ ...action, mode: { kind: "status", status: status ?? 403 } })}
            />
          )}
        </Field>
      );
    case "Script":
      return <ScriptActionForm action={action} onChange={onChange} />;
    case "WsMessage":
      return <WsMessageForm action={action} onChange={onChange} />;
    case "ServeFrom":
      return <ServeFromForm action={action} onChange={onChange} />;
  }
}

/**
 * Answers from a recording in the mock library (M20): the recorded
 * response whose request has the same method, scheme, host and path (and,
 * as chosen, query and body).
 */
function ServeFromForm({ action, onChange }: { action: Extract<Action, { type: "ServeFrom" }>; onChange: (a: Action) => void }) {
  const sources = useMockSources();
  const openSettings = useUi((s) => s.openSettings);
  const current = sources.find((s) => s.id === action.source);
  const options = [
    ...(current ? [] : [{ value: NO_SCRIPT, label: action.source ? "(missing recording)" : "Choose a recording…" }]),
    ...sources.map((s) => ({ value: s.id, label: `${s.name} · ${s.flows} flow${s.flows === 1 ? "" : "s"}` })),
  ];
  return (
    <>
      <Field label="Recording">
        <Choice
          value={current ? action.source : NO_SCRIPT}
          options={options}
          onChange={(id) => id !== NO_SCRIPT && onChange({ ...action, source: id })}
          className="w-64"
          testId="serve-source"
        />
        <Button variant="ghost" size="xs" onClick={() => openSettings("mocking")} title="The mock library, in Settings">
          Library…
        </Button>
      </Field>
      {current && current.hosts.length > 0 && (
        <div className="text-muted-foreground truncate pl-26 text-2xs" title={current.hosts.join("\n")}>
          Recorded from {current.hosts.join(", ")}
        </div>
      )}
      <Field label="Also compare">
        <Checkbox
          checked={action.key.query}
          onChange={(query) => onChange({ ...action, key: { ...action.key, query } })}
          label="Query (any order)"
          testId="serve-key-query"
        />
        <Checkbox
          checked={action.key.body}
          onChange={(body) => onChange({ ...action, key: { ...action.key, body } })}
          label="Request body"
          testId="serve-key-body"
        />
      </Field>
      <Field label="Several match">
        <Choice
          value={action.pick}
          options={(Object.keys(SERVE_PICK_LABELS) as ServePick[]).map((k) => ({ value: k, label: SERVE_PICK_LABELS[k] }))}
          onChange={(pick) => onChange({ ...action, pick })}
          className="w-40"
          testId="serve-pick"
        />
      </Field>
      <Field label="None match">
        <Choice
          value={action.on_miss}
          options={(Object.keys(SERVE_MISS_LABELS) as ServeMiss[]).map((k) => ({ value: k, label: SERVE_MISS_LABELS[k] }))}
          onChange={(on_miss) => onChange({ ...action, on_miss })}
          className="w-40"
          testId="serve-miss"
        />
        <span className="text-muted-foreground text-xs">
          {action.on_miss === "pass_through" ? "the request goes to the server" : "without contacting the server"}
        </span>
      </Field>
    </>
  );
}

const BOTH = "both";

/**
 * Acts on the text and binary messages of a WebSocket whose handshake the
 * rule matches, in one or both directions, optionally only on messages
 * whose payload matches a filter.
 */
function WsMessageForm({ action, onChange }: { action: Extract<Action, { type: "WsMessage" }>; onChange: (a: Action) => void }) {
  const { op } = action;
  const setOp = (next: typeof op) => onChange({ ...action, op: next });
  return (
    <>
      <Field label="Messages">
        <Choice
          value={action.direction ?? BOTH}
          options={[
            { value: BOTH, label: "Both ways" },
            { value: "sent", label: "Sent by the client" },
            { value: "received", label: "From the server" },
          ]}
          onChange={(v) => onChange({ ...action, direction: v === BOTH ? null : (v as WsDirection) })}
          className="w-40"
          testId="ws-direction"
        />
      </Field>
      <Field label="Containing">
        <PatternInput value={action.filter ?? null} optional placeholder="any message" onChange={(filter) => onChange({ ...action, filter })} testId="ws-filter" />
      </Field>
      <Field label="Do">
        <Choice
          value={op.kind}
          options={(Object.keys(WS_OP_LABELS) as WsOpKind[]).map((k) => ({ value: k, label: WS_OP_LABELS[k] }))}
          onChange={(kind) => setOp(defaultWsOp(kind))}
          className="w-32"
          testId="ws-op"
        />
        {op.kind === "delay" && (
          <>
            <NumberInput value={op.ms} onChange={(ms) => setOp({ ...op, ms: ms ?? 0 })} />
            <span className="text-muted-foreground text-xs">ms; later messages wait behind it</span>
          </>
        )}
        {op.kind === "drop" && <span className="text-muted-foreground text-xs">recorded, never forwarded</span>}
        {op.kind === "breakpoint" && <span className="text-muted-foreground text-xs">later messages wait behind it</span>}
      </Field>
      {op.kind === "replace" && (
        <>
          <Field label="Find">
            <PatternInput value={op.pattern} onChange={(p) => p && setOp({ ...op, pattern: p })} />
          </Field>
          <Field label="Replace with">
            <Input className={MONO} value={op.replacement} placeholder="$1 for a regex group" onChange={(e) => setOp({ ...op, replacement: e.target.value })} />
          </Field>
        </>
      )}
      {(op.kind === "json_set" || op.kind === "json_remove") && (
        <Field label="JSON pointer">
          <Input className={MONO} value={op.pointer} placeholder="/data/price" onChange={(e) => setOp({ ...op, pointer: e.target.value })} data-testid="ws-pointer" />
        </Field>
      )}
      {op.kind === "json_set" && (
        <Field label="Value (JSON)">
          <JsonValueInput value={op.raw ?? JSON.stringify(op.value)} onChange={(raw) => setOp({ ...op, value: null, raw })} />
        </Field>
      )}
    </>
  );
}

/** Picks the rewrite or mock script a rule runs; view scripts run on their own. */
function ScriptActionForm({ action, onChange }: { action: Extract<Action, { type: "Script" }>; onChange: (a: Action) => void }) {
  const scripts = useScripts().filter((s) => s.kind !== "view");
  const openScripts = useUi((s) => s.openScripts);
  const current = scripts.find((s) => s.id === action.script_id);
  const options = [
    ...(current ? [] : [{ value: NO_SCRIPT, label: action.script_id ? "(missing script)" : "Choose a script…" }]),
    ...scripts.map((s) => ({ value: s.id, label: `${s.name} · ${s.kind}${s.enabled ? "" : " (off)"}` })),
  ];
  return (
    <Field label="Script">
      <Choice
        value={current ? action.script_id : NO_SCRIPT}
        options={options}
        onChange={(id) => id !== NO_SCRIPT && onChange({ ...action, script_id: id })}
        className="w-64"
        testId="script-picker"
      />
      <Button
        variant="ghost"
        size="xs"
        onClick={() => openScripts(current ? { scriptId: current.id } : {})}
        title={current ? "Open the script" : "Open the Scripts window"}
      >
        {current ? "Edit…" : "Scripts…"}
      </Button>
    </Field>
  );
}

const NO_SCRIPT = "__none__";

function MapLocalForm({ action, onChange }: { action: Extract<Action, { type: "MapLocal" }>; onChange: (a: Action) => void }) {
  const setSource = (source: MapLocalSource) => onChange({ ...action, source });
  const { source } = action;
  return (
    <>
      <Field label="Answer from">
        <Choice
          value={source.kind}
          options={[
            { value: "inline", label: "Inline body" },
            { value: "file", label: "A file" },
            { value: "directory", label: "A directory" },
          ]}
          onChange={(kind) =>
            setSource(kind === "inline" ? { kind, body: "" } : kind === "file" ? { kind, path: "" } : { kind, root: "" })
          }
          className="w-32"
        />
        <NumberInput value={action.status} min={100} max={999} onChange={(status) => onChange({ ...action, status: status ?? 200 })} className="w-16" />
        <Input
          className={MONO}
          value={action.content_type ?? ""}
          placeholder={source.kind === "inline" ? "Content-Type" : "Content-Type (from the extension)"}
          onChange={(e) => onChange({ ...action, content_type: e.target.value || null })}
        />
      </Field>
      {source.kind === "inline" && (
        <TextArea value={source.body} rows={8} onChange={(e) => setSource({ kind: "inline", body: e.target.value })} data-testid="map-local-body" />
      )}
      {source.kind === "file" && (
        <Field label="File">
          <Input className={MONO} value={source.path} placeholder="/Users/you/mock.json" onChange={(e) => setSource({ kind: "file", path: e.target.value })} />
        </Field>
      )}
      {source.kind === "directory" && (
        <Field label="Root">
          <Input className={MONO} value={source.root} placeholder="/Users/you/site" onChange={(e) => setSource({ kind: "directory", root: e.target.value })} />
        </Field>
      )}
      {source.kind === "directory" && (
        <div className="text-muted-foreground pl-26 text-2xs">
          The request path is looked up under the root, then with leading segments dropped; a path ending in / serves
          index.html.
        </div>
      )}
    </>
  );
}

/** Exact JSON text (big numbers and `1.50` kept as typed), applied once it parses. */
function JsonValueInput({ value, onChange }: { value: string; onChange: (raw: string) => void }) {
  const [text, setText] = useState(value);
  const error = jsonError(text);
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
      <Input
        className={MONO}
        value={text}
        aria-invalid={error !== null}
        placeholder='"text", 42, true, null, {"a": 1}'
        onChange={(e) => {
          setText(e.target.value);
          if (jsonError(e.target.value) === null) onChange(e.target.value.trim());
        }}
        data-testid="json-set-raw"
      />
      {error !== null && <span className="text-2xs text-[var(--s-failed)]">Not JSON yet: {error}</span>}
    </div>
  );
}

function jsonError(text: string): string | null {
  try {
    parseLossless(text);
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
}
