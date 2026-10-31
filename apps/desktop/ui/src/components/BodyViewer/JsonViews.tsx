import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronDown, ChevronRight } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { BodyItems } from "@/components/menus/body";
import { SelectionItem } from "@/components/menus/items";
import { useMenuTarget } from "@/components/menus/useMenuTarget";
import { Input } from "@/components/ui/input";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { evaluatePath, toJsonPath, toPointer, type PathSegment } from "@/lib/body/jsonPath";
import { isJsonNumber, numberText, stringifyLossless } from "@/lib/body/losslessJson";
import { copy } from "@/lib/clipboard";
import { cn } from "@/lib/utils";

import { CodeView } from "./CodeView";
import { Highlighted, MatchBar } from "./search";
import { MATCH_LIMIT, searchRegExp, useBodySearch } from "./searchTerms";
import { Centered, Note } from "./parts";
import { jsonInWorker, objectId } from "./workers/client";

/** Pretty-printed JSON in CodeMirror; large bodies are formatted in a worker. */
export function JsonPretty({ bytes, highlight }: { bytes: Uint8Array; highlight: boolean }) {
  const pretty = useQuery({
    queryKey: ["json-pretty", objectId(bytes)],
    queryFn: async () => (await jsonInWorker(bytes, "pretty")) as string,
    gcTime: 0,
  });
  if (pretty.isPending) return <Centered>Formatting…</Centered>;
  if (pretty.isError) return <InvalidJson bytes={bytes} error={pretty.error} />;
  return <CodeView text={pretty.data} language="json" highlight={highlight} />;
}

function InvalidJson({ bytes, error }: { bytes: Uint8Array; error: Error }) {
  const text = useMemo(() => new TextDecoder().decode(bytes), [bytes]);
  return (
    <div className="flex h-full flex-col">
      <Note tone="warn">Not valid JSON: {error.message}. Showing the text.</Note>
      <div className="min-h-0 flex-1">
        <CodeView text={text} language="json" />
      </div>
    </div>
  );
}

interface TreeRow {
  /** Key in the parent, or the label of a filter match at depth 0. */
  label: string | number | null;
  value: unknown;
  depth: number;
  path: PathSegment[];
  ptr: string;
  container: "object" | "array" | null;
  size: number;
  expanded: boolean;
  /** A filter result rather than a real child. */
  match: boolean;
}

const ROW = 20;

function containerOf(v: unknown): "object" | "array" | null {
  if (Array.isArray(v)) return "array";
  return v !== null && typeof v === "object" && !isJsonNumber(v) ? "object" : null;
}

function flatten(roots: { label: string | null; path: PathSegment[]; value: unknown; match: boolean }[], expanded: Set<string>) {
  const rows: TreeRow[] = [];
  const visit = (label: TreeRow["label"], value: unknown, depth: number, path: PathSegment[], match: boolean) => {
    const container = containerOf(value);
    const ptr = toPointer(path);
    const size = container === "array" ? (value as unknown[]).length : container ? Object.keys(value as object).length : 0;
    const open = container !== null && expanded.has(ptr);
    rows.push({ label, value, depth, path, ptr, container, size, expanded: open, match });
    if (!open) return;
    if (container === "array") (value as unknown[]).forEach((v, i) => visit(i, v, depth + 1, [...path, i], false));
    else for (const [k, v] of Object.entries(value as Record<string, unknown>)) visit(k, v, depth + 1, [...path, k], false);
  };
  for (const r of roots) visit(r.label, r.value, 0, r.path, r.match);
  return rows;
}

/**
 * Pointers of the keys and scalar values matching `re`, in document order
 * (M17: the filter's body terms), up to [`MATCH_LIMIT`].
 */
function treeMatches(value: unknown, re: RegExp): string[] {
  const out: string[] = [];
  const test = (s: string) => {
    re.lastIndex = 0;
    return re.test(s);
  };
  const visit = (v: unknown, path: PathSegment[], key: string | null) => {
    if (out.length >= MATCH_LIMIT) return;
    const container = containerOf(v);
    if ((key !== null && test(key)) || (!container && test(valueText(v)))) out.push(toPointer(path));
    if (container === "array") (v as unknown[]).forEach((c, i) => visit(c, [...path, i], null));
    else if (container) for (const [k, c] of Object.entries(v as Record<string, unknown>)) visit(c, [...path, k], k);
  };
  visit(value, [], null);
  return out;
}

/** The containers above a pointer: `/a/0/b` → "", `/a`, `/a/0`. */
function ancestors(ptr: string): string[] {
  const parts = ptr.split("/").slice(1);
  return parts.map((_, i) => (i === 0 ? "" : `/${parts.slice(0, i).join("/")}`));
}

/** Pointers of every container down to `depth`, for the initial expansion. */
function expandedTo(value: unknown, depth: number, path: PathSegment[] = [], out = new Set<string>()): Set<string> {
  const container = containerOf(value);
  if (!container || depth < 0) return out;
  out.add(toPointer(path));
  const entries = container === "array" ? (value as unknown[]).map((v, i) => [i, v] as const) : Object.entries(value as object);
  // Don't walk huge containers just to pre-expand them.
  if (entries.length > 500) return out;
  for (const [k, v] of entries) expandedTo(v, depth - 1, [...path, k], out);
  return out;
}

/**
 * The JSON tree: virtualised and collapsible, with type-coloured values,
 * child counts, a JSONPath / JSON pointer filter and right-click copy.
 * Parsed losslessly, so numbers show (and copy) exactly as sent.
 */
export function JsonTree({ bytes }: { bytes: Uint8Array }) {
  const parsed = useQuery({
    queryKey: ["json-parse", objectId(bytes)],
    queryFn: () => jsonInWorker(bytes, "parse"),
    gcTime: 0,
  });
  if (parsed.isPending) return <Centered>Parsing…</Centered>;
  if (parsed.isError) return <InvalidJson bytes={bytes} error={parsed.error} />;
  return <Tree root={parsed.data} />;
}

/** Notes on values by JSON pointer: wire types, `bytes`, ext types, tags. */
export type TreeBadges = ReadonlyMap<string, readonly string[]>;

/**
 * The tree over an already parsed value, with `badges` after the values
 * they note (decoded protobuf, MessagePack and CBOR).
 */
export function Tree({ root, badges, testId = "json-tree" }: { root: unknown; badges?: TreeBadges; testId?: string }) {
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState(() => expandedTo(root, 1));
  const scroller = useRef<HTMLDivElement>(null);

  const result = useMemo(() => evaluatePath(root, filter), [root, filter]);
  const roots = useMemo(() => {
    if ("error" in result) return [{ label: null, path: [], value: root, match: false }];
    if (filter.trim() === "" || filter.trim() === "$") return [{ label: null, path: [], value: root, match: false }];
    return result.matches.map((m) => ({ label: toJsonPath(m.path), path: m.path, value: m.value, match: true }));
  }, [result, filter, root]);
  const rows = useMemo(() => flatten(roots, expanded), [roots, expanded]);

  // The filter's body terms (M17): marked in keys and values, stepped through.
  const terms = useBodySearch();
  const termsKey = terms.join("\u0000");
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed by the terms' text, not the array's identity.
  const re = useMemo(() => searchRegExp(terms), [termsKey]);
  const hits = useMemo(() => (re ? treeMatches(root, re) : []), [re, root]);
  const [stepped, setStepped] = useState<{ hits: string[]; index: number } | null>(null);
  const current = stepped?.hits === hits ? stepped.index : -1;
  const pendingScroll = useRef<string | null>(null);
  const reveal = useCallback((ptr: string) => {
    pendingScroll.current = ptr;
    setExpanded((prev) => {
      const missing = ancestors(ptr).filter((a) => !prev.has(a));
      return missing.length === 0 ? prev : new Set([...prev, ...missing]);
    });
  }, []);
  const step = (direction: 1 | -1) => {
    if (hits.length === 0) return;
    const next = current < 0 ? (direction === 1 ? 0 : hits.length - 1) : (current + direction + hits.length) % hits.length;
    setStepped({ hits, index: next });
    setFilter("");
    reveal(hits[next]!);
  };

  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual is not compiler-safe; rows re-render through their props.
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW,
    overscan: 20,
  });

  const toggle = useCallback((ptr: string, deep: boolean, value?: unknown) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(ptr) && !deep) next.delete(ptr);
      else if (deep && value !== undefined) {
        // The whole subtree, bounded by expandedTo's size guard.
        for (const p of expandedTo(value, 64)) next.add(ptr + p);
      } else next.add(ptr);
      return next;
    });
  }, []);

  // Blank space still has the selection and the body items.
  const menu = useMenuTarget((key) => rows[Number(key)], { always: true });
  const menuRow = menu.target;

  const matchCount = "matches" in result && roots[0]?.match ? result.matches.length : null;
  const currentPtr = current >= 0 ? hits[current] : undefined;
  // Scroll to a revealed match once its row exists.
  useEffect(() => {
    const ptr = pendingScroll.current;
    if (ptr === null) return;
    const index = rows.findIndex((r) => r.ptr === ptr && !r.match);
    if (index >= 0) {
      pendingScroll.current = null;
      virtualizer.scrollToIndex(index, { align: "center" });
    }
  }, [rows, virtualizer]);
  return (
    <div className="flex h-full flex-col" data-testid={testId}>
      <div className="flex h-7 shrink-0 items-center gap-2 border-b px-2">
        <Input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter: $.items[*].id  or  /items/0"
          className="h-5 flex-1 font-mono text-[11px]"
          spellCheck={false}
          data-testid="json-filter"
        />
        <span className={cn("text-[11px]", "error" in result ? "text-[var(--s-failed)]" : "text-muted-foreground")}>
          {"error" in result ? result.error : matchCount !== null ? `${matchCount} match${matchCount === 1 ? "" : "es"}` : ""}
        </span>
        {re && <MatchBar count={hits.length} current={current} onStep={step} className="border-0 shadow-none" />}
      </div>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div ref={scroller} className="min-h-0 flex-1 overflow-auto font-mono text-[11.5px]" onContextMenu={menu.onContextMenu} data-selectable>
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((item) => (
                <TreeLine
                  key={item.key}
                  index={item.index}
                  row={rows[item.index]!}
                  top={item.start}
                  onToggle={toggle}
                  badges={badges?.get(rows[item.index]!.ptr)}
                  re={re}
                  current={currentPtr !== undefined && rows[item.index]!.ptr === currentPtr}
                />
              ))}
            </div>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="min-w-44 text-xs" data-testid="json-tree-menu">
          <SelectionItem text={menu.selection} />
          {menuRow && (
            <>
              <ContextMenuItem onSelect={() => void copy(valueText(menuRow.value), "value")}>Copy value</ContextMenuItem>
              {menuRow.label !== null && !menuRow.match && (
                <ContextMenuItem onSelect={() => void copy(String(menuRow.label), "key")}>Copy key</ContextMenuItem>
              )}
              <ContextMenuItem onSelect={() => void copy(toJsonPath(menuRow.path), "path")}>Copy path</ContextMenuItem>
              <ContextMenuItem onSelect={() => void copy(toPointer(menuRow.path) || "/", "JSON pointer")}>Copy JSON pointer</ContextMenuItem>
              {menuRow.container && (
                <>
                  <ContextMenuSeparator />
                  <ContextMenuItem onSelect={() => toggle(menuRow.ptr, true, menuRow.value)}>Expand all below</ContextMenuItem>
                  <ContextMenuItem onSelect={() => setExpanded(new Set())}>Collapse all</ContextMenuItem>
                </>
              )}
            </>
          )}
          <BodyItems separator={menuRow !== null} />
        </ContextMenuContent>
      </ContextMenu>
    </div>
  );
}

function valueText(v: unknown): string {
  return typeof v === "string" ? v : stringifyLossless(v);
}

const TreeLine = memo(function TreeLine({
  index,
  row,
  top,
  onToggle,
  badges,
  re,
  current,
}: {
  index: number;
  row: TreeRow;
  top: number;
  onToggle: (ptr: string, deep: boolean, value?: unknown) => void;
  badges?: readonly string[];
  re: RegExp | null;
  current: boolean;
}) {
  const Caret = row.expanded ? ChevronDown : ChevronRight;
  const expandable = row.container !== null && row.size > 0;
  return (
    <div
      data-row={index}
      data-menu-key={index}
      data-current-match={current || undefined}
      className={cn(
        "absolute left-0 flex w-full items-center gap-1 pr-2 whitespace-nowrap hover:bg-[var(--code-row-hover)]",
        current && "bg-[var(--code-active-line)]",
      )}
      style={{ top, height: ROW, paddingLeft: 6 + row.depth * 14 }}
      onClick={expandable ? (e) => onToggle(row.ptr, e.altKey, row.value) : undefined}
    >
      {expandable ? <Caret className="text-muted-foreground size-3 shrink-0" /> : <span className="w-3 shrink-0" />}
      {row.label !== null && (
        <span>
          <span className={row.match ? "text-muted-foreground" : typeof row.label === "number" ? "text-[var(--syntax-punct)]" : "text-[var(--syntax-key)]"}>
            {row.match || typeof row.label === "number" ? (
              row.label
            ) : (
              <Highlighted text={JSON.stringify(row.label)} re={re} current={current} />
            )}
          </span>
          <span className="text-[var(--syntax-punct)]">:</span>
        </span>
      )}
      <Value row={row} re={re} current={current} />
      {badges?.map((b) => (
        <span
          key={b}
          className="text-muted-foreground ml-1 shrink-0 rounded-sm border px-1 font-sans text-[10px] leading-[14px]"
          data-testid="tree-badge"
        >
          {b}
        </span>
      ))}
    </div>
  );
});

function Value({ row, re, current }: { row: TreeRow; re: RegExp | null; current: boolean }) {
  if (row.container) {
    const [open, close] = row.container === "array" ? ["[", "]"] : ["{", "}"];
    const unit = row.container === "array" ? "item" : "key";
    if (row.size === 0) return <span className="text-[var(--syntax-punct)]">{open + close}</span>;
    return (
      <>
        <span className="text-[var(--syntax-punct)]">{row.expanded ? open : `${open}…${close}`}</span>
        <span className="text-muted-foreground ml-1 text-[10.5px]">
          {row.size} {unit}
          {row.size === 1 ? "" : "s"}
        </span>
      </>
    );
  }
  const v = row.value;
  if (typeof v === "string")
    return (
      <span className="truncate text-[var(--syntax-string)]">
        <Highlighted text={JSON.stringify(v)} re={re} current={current} />
      </span>
    );
  if (typeof v === "number" || isJsonNumber(v))
    return (
      <span className="text-[var(--syntax-number)]">
        <Highlighted text={numberText(v)} re={re} current={current} />
      </span>
    );
  return (
    <span className="text-[var(--syntax-literal)]">
      <Highlighted text={String(v)} re={re} current={current} />
    </span>
  );
}
