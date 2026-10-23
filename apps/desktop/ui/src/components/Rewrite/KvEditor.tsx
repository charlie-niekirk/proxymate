import { Plus, RotateCcw, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { newId } from "@/lib/json/doc";
import type { KvRow } from "@/lib/rewrite/diff";
import { cn } from "@/lib/utils";

/**
 * A name/value table for headers, query parameters and form fields, with
 * rows tinted where they differ from `original` and removed rows listed
 * (struck through) so they can be put back.
 */
export function KvEditor({
  rows,
  original,
  onChange,
  fold = false,
  readOnlyName,
  repeatedReadOnly = false,
  addLabel,
  testId,
}: {
  rows: KvRow[];
  original: readonly KvRow[];
  onChange: (rows: KvRow[]) => void;
  /** Names compare case-insensitively (headers). */
  fold?: boolean;
  /** Names whose rows can't be edited or removed (the framing headers). */
  readOnlyName?: (name: string) => string | null;
  /** A name the original repeats can be removed but not edited (query, form). */
  repeatedReadOnly?: boolean;
  addLabel: string;
  testId: string;
}) {
  const norm = (n: string) => (fold ? n.toLowerCase() : n);
  const before = new Map(original.map((r) => [r.id, r]));
  const counts = new Map<string, number>();
  for (const r of original) counts.set(norm(r.name), (counts.get(norm(r.name)) ?? 0) + 1);
  const ids = new Set(rows.map((r) => r.id));
  const removed = original.filter((r) => !ids.has(r.id));
  const update = (id: number, patch: Partial<KvRow>) => onChange(rows.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  const names = new Map<string, number>();
  for (const r of rows) names.set(norm(r.name), (names.get(norm(r.name)) ?? 0) + 1);

  return (
    <div className="flex flex-col gap-0.5 p-2 text-xs" data-testid={testId}>
      {rows.map((r) => {
        const was = before.get(r.id);
        const locked = readOnlyName?.(r.name) ?? null;
        const repeated = repeatedReadOnly && was !== undefined && (counts.get(norm(was.name)) ?? 0) > 1;
        const status = !was ? "added" : was.name !== r.name || was.value !== r.value ? "changed" : "same";
        const duplicate = repeatedReadOnly && !repeated && r.name !== "" && (names.get(norm(r.name)) ?? 0) > 1;
        return (
          <div
            key={r.id}
            data-status={status}
            className="flex items-center gap-1 rounded-sm px-0.5 data-[status=added]:bg-[var(--diff-add-bg)] data-[status=changed]:bg-[var(--diff-change-bg)]"
            data-testid="kv-row"
          >
            <Input
              value={r.name}
              disabled={locked !== null || repeated || was !== undefined}
              title={was !== undefined ? "Remove the row and add a new one to rename it" : undefined}
              onChange={(e) => update(r.id, { name: e.target.value })}
              placeholder="name"
              className="h-6 w-56 shrink-0 font-mono text-xs"
              data-testid="kv-name"
            />
            <Input
              value={r.value}
              disabled={locked !== null || repeated}
              aria-invalid={duplicate}
              title={locked ?? (repeated ? "Repeated: it can be removed, not edited" : duplicate ? "This name is already used" : undefined)}
              onChange={(e) => update(r.id, { value: e.target.value })}
              placeholder="value"
              className="h-6 font-mono text-xs"
              data-testid="kv-value"
            />
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label="Remove"
              disabled={locked !== null}
              onClick={() => onChange(rows.filter((x) => x.id !== r.id))}
              data-testid="kv-remove"
            >
              <X />
            </Button>
          </div>
        );
      })}
      {removed.map((r) => (
        <div key={r.id} className="flex items-center gap-1 rounded-sm bg-[var(--diff-del-bg)] px-0.5" data-testid="kv-removed">
          <span className="w-56 shrink-0 truncate px-2 font-mono text-[var(--diff-del)] line-through">{r.name}</span>
          <span className="min-w-0 flex-1 truncate px-2 font-mono text-[var(--diff-del)] line-through">{r.value}</span>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Put back"
            title="Put back"
            onClick={() => {
              // Back in its original place, among the rows that are still there.
              const order = original.map((o) => o.id);
              const next = [...rows, r].sort((a, b) => rank(order, a.id) - rank(order, b.id));
              onChange(next);
            }}
          >
            <RotateCcw />
          </Button>
        </div>
      ))}
      <Button
        variant="ghost"
        size="xs"
        className={cn("text-muted-foreground self-start")}
        onClick={() => onChange([...rows, { id: newId(), name: "", value: "" }])}
        data-testid={`${testId}-add`}
      >
        <Plus />
        {addLabel}
      </Button>
    </div>
  );
}

/** Original rows by their place; new ones after them, in order. */
function rank(order: number[], id: number): number {
  const i = order.indexOf(id);
  return i < 0 ? order.length + id : i;
}
