import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, Redo2, Undo2 } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { copyText } from "@/components/BodyViewer/util";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import type { Undoable } from "@/hooks/useUndoable";
import {
  blank,
  children,
  convert,
  duplicate,
  indexDoc,
  insert,
  isContainer,
  isJsonNumber,
  nodeAt,
  pointerOf,
  remove,
  removedCount,
  rename,
  ROOT_ID,
  setValue,
  statusOf,
  toJson,
  type DocIndex,
  type JNode,
  type JType,
  type Path,
  type RowStatus,
} from "@/lib/json/doc";
import { cn } from "@/lib/utils";

const ROW = 20;

const TYPES: { t: JType; label: string }[] = [
  { t: "str", label: "String" },
  { t: "num", label: "Number" },
  { t: "bool", label: "Boolean" },
  { t: "null", label: "Null" },
  { t: "obj", label: "Object" },
  { t: "arr", label: "Array" },
];

interface Row {
  path: Path;
  id: number;
  depth: number;
  /** Key in an object, index in an array, null for the root. */
  label: string | number | null;
  node: JNode;
  status: RowStatus;
  /** Original children no longer there. */
  removed: number;
  expanded: boolean;
}

function flatten(root: JNode, expanded: ReadonlySet<number>, original: { index: DocIndex; root: JNode } | null): Row[] {
  const rows: Row[] = [];
  const visit = (node: JNode, path: Path, id: number, label: Row["label"], status: RowStatus, depth: number) => {
    const open = isContainer(node) && expanded.has(id);
    rows.push({
      path,
      id,
      depth,
      label,
      node,
      status,
      removed: original && isContainer(node) ? removedCount(node, id, original.index, original.root) : 0,
      expanded: open,
    });
    if (!open) return;
    children(node).forEach((c, i) => {
      const s = original ? statusOf(c, original.index) : "same";
      visit(c.v, [...path, c.id], c.id, "key" in c ? c.key : i, s === "same" && status === "added" ? "added" : s, depth + 1);
    });
  };
  const rootStatus: RowStatus = original && original.root.t !== root.t ? "changed" : "same";
  visit(root, [], ROOT_ID, null, rootStatus, 0);
  return rows;
}

/** Ids of the containers down to `depth`, for the first expansion. */
function expandedTo(node: JNode, depth: number, id = ROOT_ID, out = new Set<number>()): Set<number> {
  if (!isContainer(node) || depth < 0) return out;
  out.add(id);
  const kids = children(node);
  if (kids.length > 500) return out;
  for (const c of kids) expandedTo(c.v, depth - 1, c.id, out);
  return out;
}

const samePath = (a: Path | null, b: Path | null) => !!a && !!b && a.length === b.length && a.every((x, i) => x === b[i]);

/** Radix dialogs close on Escape before an inline editor sees it; pass this as their `onEscapeKeyDown`. */
export function keepEscapeInEditor(e: KeyboardEvent | globalThis.KeyboardEvent) {
  const target = e.target instanceof HTMLElement ? e.target : null;
  if (target?.closest("[data-json-editor] input")) e.preventDefault();
}

/**
 * The editable JSON tree: virtualised, with typed inline editors, a
 * change-type menu, add / remove / rename / duplicate / reset, undo and
 * redo, and rows tinted where they differ from `original`.
 *
 * Keys: ↑↓ move, ←→ collapse and expand, ↩ edit the value, F2 rename,
 * ⌘↩ add after, ⌘D duplicate, ⌫ remove, ⌘Z / ⇧⌘Z undo and redo, Esc
 * cancels an edit.
 */
export function JsonEditor({
  doc,
  original,
  replaced,
  toolbar,
  testId = "json-editor",
}: {
  doc: Undoable<JNode>;
  /** What the rows are compared with; null shows no changes. */
  original: JNode | null;
  /** Ids of arrays a rewrite sets as a whole (badged). */
  replaced?: ReadonlySet<number>;
  /** Extra controls at the right of the toolbar. */
  toolbar?: ReactNode;
  testId?: string;
}) {
  const root = doc.value;
  const [expanded, setExpanded] = useState(() => expandedTo(root, 1));
  const [selected, setSelected] = useState<Path | null>([]);
  const [editing, setEditing] = useState<{ path: Path; field: "key" | "value" } | null>(null);
  const [menuRow, setMenuRow] = useState<Row | null>(null);
  const scroller = useRef<HTMLDivElement>(null);

  const reference = useMemo(() => (original ? { index: indexDoc(original), root: original } : null), [original]);
  const rows = useMemo(() => flatten(root, expanded, reference), [root, expanded, reference]);
  const selectedIndex = rows.findIndex((r) => samePath(r.path, selected));

  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual is not compiler-safe; rows re-render through their props.
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW,
    overscan: 20,
  });

  const focus = useCallback(() => scroller.current?.focus({ preventScroll: true }), []);
  const select = useCallback(
    (path: Path | null, scroll = true) => {
      setSelected(path);
      if (!scroll || !path) return;
      const i = rows.findIndex((r) => samePath(r.path, path));
      if (i >= 0) virtualizer.scrollToIndex(i, { align: "auto" });
    },
    [rows, virtualizer],
  );

  // A selection that an undo removed falls back to the root.
  useEffect(() => {
    if (selected && selected.length > 0 && !nodeAt(root, selected)) setSelected([]);
  }, [root, selected]);

  const toggle = useCallback((id: number, open?: boolean) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (open ?? !next.has(id)) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  // ---- Edits ----

  const change = useCallback((next: JNode) => doc.set(next), [doc]);

  const addAfter = (row: Row, t: JType = "str") => {
    if (row.path.length === 0) return addChild(row, t);
    const parent = row.path.slice(0, -1);
    const { root: next, path } = insert(root, parent, row.id, blank(t));
    change(next);
    select(path);
    const inObject = nodeAt(root, parent)?.t === "obj";
    setEditing({ path, field: inObject ? "key" : "value" });
  };

  const addChild = (row: Row, t: JType = "str") => {
    if (!isContainer(row.node)) return;
    const { root: next, path } = insert(root, row.path, null, blank(t));
    change(next);
    toggle(row.id, true);
    select(path);
    setEditing({ path, field: row.node.t === "obj" ? "key" : "value" });
  };

  const removeRow = (row: Row) => {
    if (row.path.length === 0) return;
    const i = rows.indexOf(row);
    change(remove(root, row.path));
    const next = rows.slice(i + 1).find((r) => r.depth <= row.depth) ?? rows[i - 1];
    setSelected(next && !samePath(next.path, row.path) ? next.path : row.path.slice(0, -1));
    focus();
  };

  const duplicateRow = (row: Row) => {
    if (row.path.length === 0) return;
    const { root: next, path } = duplicate(root, row.path);
    change(next);
    select(path);
  };

  const changeType = (row: Row, t: JType) => {
    change(setValue(root, row.path, convert(row.node, t)));
    if (isContainer(convert(row.node, t))) toggle(row.id, true);
  };

  const reset = (row: Row) => {
    if (!reference) return;
    if (row.path.length === 0) return change(reference.root);
    const was = reference.index.byId.get(row.id);
    if (!was) return removeRow(row);
    let next = setValue(root, row.path, was.member.v);
    if ("key" in was.member) next = rename(next, row.path, was.member.key);
    change(next);
  };

  const commit = (row: Row, field: "key" | "value", text: string): string | null => {
    if (field === "key") {
      const parent = nodeAt(root, row.path.slice(0, -1));
      if (parent?.t === "obj" && parent.entries.some((e) => e.key === text && e.id !== row.id)) return `${JSON.stringify(text)} is already a key here`;
      change(rename(root, row.path, text));
    } else if (row.node.t === "num") {
      if (!isJsonNumber(text.trim())) return "Not a JSON number";
      change(setValue(root, row.path, { t: "num", raw: text.trim() }));
    } else if (row.node.t === "str") {
      change(setValue(root, row.path, { t: "str", v: text }));
    }
    setEditing(null);
    focus();
    return null;
  };

  const startEdit = (row: Row, field: "key" | "value") => {
    if (field === "key" && typeof row.label !== "string") return;
    if (field === "value") {
      if (row.node.t === "bool") return change(setValue(root, row.path, { t: "bool", v: !row.node.v }));
      if (isContainer(row.node)) return toggle(row.id);
      if (row.node.t === "null") return;
    }
    setSelected(row.path);
    setEditing({ path: row.path, field });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (editing || e.target !== scroller.current) return;
    const mod = e.metaKey || e.ctrlKey;
    const row = rows[selectedIndex] ?? rows[0];
    if (!row) return;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const handled = (() => {
      if (mod) {
        if (key === "z" && e.shiftKey) doc.redo();
        else if (key === "z") doc.undo();
        else if (key === "y") doc.redo();
        else if (key === "d") duplicateRow(row);
        else if (key === "Enter") addAfter(row);
        else return false;
        return true;
      }
      switch (key) {
        case "ArrowDown":
          select(rows[Math.min(rows.length - 1, selectedIndex + 1)]?.path ?? null);
          return true;
        case "ArrowUp":
          select(rows[Math.max(0, selectedIndex - 1)]?.path ?? null);
          return true;
        case "Home":
          select(rows[0]?.path ?? null);
          return true;
        case "End":
          select(rows[rows.length - 1]?.path ?? null);
          return true;
        case "ArrowRight":
          if (isContainer(row.node) && !row.expanded && children(row.node).length > 0) toggle(row.id, true);
          else if (row.expanded) select(rows[selectedIndex + 1]?.path ?? null);
          return true;
        case "ArrowLeft":
          if (row.expanded) toggle(row.id, false);
          else if (row.path.length > 0) select(row.path.slice(0, -1));
          return true;
        case "Enter":
          startEdit(row, "value");
          return true;
        case "F2":
          startEdit(row, "key");
          return true;
        case "Backspace":
        case "Delete":
          removeRow(row);
          return true;
      }
      return false;
    })();
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  const onContextMenu = (e: React.MouseEvent) => {
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-row]");
    const row = el ? (rows[Number(el.dataset.row)] ?? null) : null;
    setMenuRow(row);
    if (row) setSelected(row.path);
  };

  const pointer = (row: Row) => {
    const tokens: (string | number)[] = [];
    let node = root;
    for (const id of row.path) {
      const kids = children(node);
      const i = kids.findIndex((c) => c.id === id);
      const c = kids[i]!;
      tokens.push("key" in c ? c.key : i);
      node = c.v;
    }
    return pointerOf(tokens);
  };

  const menuParent = menuRow && menuRow.path.length > 0 ? nodeAt(root, menuRow.path.slice(0, -1)) : null;
  const changed = menuRow && reference && (menuRow.status !== "same" || menuRow.removed > 0 || (menuRow.path.length === 0 && toJson(root) !== toJson(reference.root)));
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid={testId}>
      <div className="flex h-7 shrink-0 items-center gap-0.5 border-b px-1.5 text-[11px]">
        <Button variant="ghost" size="icon-xs" title="Undo (⌘Z)" aria-label="Undo" disabled={!doc.canUndo} onClick={() => doc.undo()} data-testid="json-undo">
          <Undo2 />
        </Button>
        <Button variant="ghost" size="icon-xs" title="Redo (⇧⌘Z)" aria-label="Redo" disabled={!doc.canRedo} onClick={() => doc.redo()} data-testid="json-redo">
          <Redo2 />
        </Button>
        <Button variant="ghost" size="icon-xs" title="Expand all" aria-label="Expand all" onClick={() => setExpanded(expandedTo(root, 64))}>
          <ChevronsUpDown />
        </Button>
        <Button variant="ghost" size="icon-xs" title="Collapse all" aria-label="Collapse all" onClick={() => setExpanded(new Set([ROOT_ID]))}>
          <ChevronsDownUp />
        </Button>
        <span className="text-muted-foreground ml-1 truncate">↩ edit · F2 rename · ⌘↩ add · ⌫ remove</span>
        <div className="ml-auto flex items-center gap-1.5">{toolbar}</div>
      </div>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            ref={scroller}
            tabIndex={0}
            className="min-h-0 flex-1 overflow-auto font-mono text-[11.5px] outline-none"
            onKeyDown={onKeyDown}
            onContextMenu={onContextMenu}
            data-json-editor
            data-testid={`${testId}-rows`}
          >
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((item) => {
                const row = rows[item.index]!;
                const edit = editing && samePath(editing.path, row.path) ? editing.field : null;
                return (
                  <Line
                    key={item.key}
                    index={item.index}
                    row={row}
                    top={item.start}
                    selected={item.index === selectedIndex}
                    replaced={replaced?.has(row.id) ?? false}
                    editing={edit}
                    onSelect={(path) => {
                      setSelected(path);
                      focus();
                    }}
                    onToggle={toggle}
                    onStartEdit={startEdit}
                    onCommit={commit}
                    onCancel={() => {
                      setEditing(null);
                      focus();
                    }}
                  />
                );
              })}
            </div>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="min-w-48 text-xs" data-testid="json-editor-menu">
          {menuRow && (
            <>
              {(menuRow.node.t === "str" || menuRow.node.t === "num") && (
                <ContextMenuItem onSelect={() => startEdit(menuRow, "value")}>
                  Edit value<ContextMenuShortcut>↩</ContextMenuShortcut>
                </ContextMenuItem>
              )}
              {menuParent?.t === "obj" && (
                <ContextMenuItem onSelect={() => startEdit(menuRow, "key")} data-testid="json-menu-rename">
                  Rename<ContextMenuShortcut>F2</ContextMenuShortcut>
                </ContextMenuItem>
              )}
              <ContextMenuSub>
                <ContextMenuSubTrigger data-testid="json-menu-type">Change type</ContextMenuSubTrigger>
                <ContextMenuSubContent className="text-xs">
                  {TYPES.map(({ t, label }) => (
                    <ContextMenuItem key={t} disabled={menuRow.node.t === t} onSelect={() => changeType(menuRow, t)} data-testid={`json-type-${t}`}>
                      {label}
                    </ContextMenuItem>
                  ))}
                </ContextMenuSubContent>
              </ContextMenuSub>
              <ContextMenuSeparator />
              {isContainer(menuRow.node) && (
                <AddMenu label={menuRow.node.t === "obj" ? "Add key" : "Add item"} onAdd={(t) => addChild(menuRow, t)} testId="json-menu-add-child" />
              )}
              {menuRow.path.length > 0 && <AddMenu label="Add after" shortcut="⌘↩" onAdd={(t) => addAfter(menuRow, t)} testId="json-menu-add-after" />}
              {menuRow.path.length > 0 && (
                <ContextMenuItem onSelect={() => duplicateRow(menuRow)}>
                  Duplicate<ContextMenuShortcut>⌘D</ContextMenuShortcut>
                </ContextMenuItem>
              )}
              {changed && (
                <ContextMenuItem onSelect={() => reset(menuRow)} data-testid="json-menu-reset">
                  Reset to original
                </ContextMenuItem>
              )}
              {menuRow.path.length > 0 && (
                <ContextMenuItem variant="destructive" onSelect={() => removeRow(menuRow)} data-testid="json-menu-remove">
                  Remove<ContextMenuShortcut>⌫</ContextMenuShortcut>
                </ContextMenuItem>
              )}
              <ContextMenuSeparator />
              <ContextMenuItem onSelect={() => void copyText(menuRow.node.t === "str" ? menuRow.node.v : toJson(menuRow.node, 2))}>Copy value</ContextMenuItem>
              <ContextMenuItem onSelect={() => void copyText(pointer(menuRow) || "/")}>Copy JSON pointer</ContextMenuItem>
            </>
          )}
        </ContextMenuContent>
      </ContextMenu>
    </div>
  );
}

function AddMenu({ label, shortcut, onAdd, testId }: { label: string; shortcut?: string; onAdd: (t: JType) => void; testId: string }) {
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger data-testid={testId}>
        {label}
        {shortcut && <ContextMenuShortcut>{shortcut}</ContextMenuShortcut>}
      </ContextMenuSubTrigger>
      <ContextMenuSubContent className="text-xs">
        {TYPES.map(({ t, label: name }) => (
          <ContextMenuItem key={t} onSelect={() => onAdd(t)} data-testid={`${testId}-${t}`}>
            {name}
          </ContextMenuItem>
        ))}
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

const Line = memo(function Line({
  index,
  row,
  top,
  selected,
  replaced,
  editing,
  onSelect,
  onToggle,
  onStartEdit,
  onCommit,
  onCancel,
}: {
  index: number;
  row: Row;
  top: number;
  selected: boolean;
  replaced: boolean;
  editing: "key" | "value" | null;
  onSelect: (path: Path) => void;
  onToggle: (id: number) => void;
  onStartEdit: (row: Row, field: "key" | "value") => void;
  onCommit: (row: Row, field: "key" | "value", text: string) => string | null;
  onCancel: () => void;
}) {
  const Caret = row.expanded ? ChevronDown : ChevronRight;
  const expandable = isContainer(row.node) && children(row.node).length > 0;
  return (
    <div
      data-row={index}
      data-status={row.status}
      data-selected={selected}
      data-testid="json-editor-row"
      className={cn(
        "absolute left-0 flex w-full items-center gap-1 pr-2 whitespace-nowrap",
        "hover:bg-[var(--code-row-hover)] data-[status=added]:bg-[var(--diff-add-bg)] data-[status=changed]:bg-[var(--diff-change-bg)] data-[status=renamed]:bg-[var(--diff-change-bg)]",
        "data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground",
      )}
      style={{ top, height: ROW, paddingLeft: 6 + row.depth * 14 }}
      onMouseDown={() => onSelect(row.path)}
    >
      {expandable ? (
        <Caret
          className="text-muted-foreground size-3 shrink-0"
          onMouseDown={(e) => {
            e.stopPropagation();
            onToggle(row.id);
          }}
        />
      ) : (
        <span className="w-3 shrink-0" />
      )}
      {row.label !== null &&
        (editing === "key" ? (
          <InlineInput initial={String(row.label)} onCommit={(t) => onCommit(row, "key", t)} onCancel={onCancel} testId="json-key-input" />
        ) : (
          <span
            className={cn(typeof row.label === "number" ? "text-[var(--syntax-punct)]" : "text-[var(--syntax-key)]", row.status === "renamed" && "underline decoration-dotted")}
            onDoubleClick={() => onStartEdit(row, "key")}
          >
            {typeof row.label === "number" ? row.label : JSON.stringify(row.label)}
          </span>
        ))}
      {row.label !== null && <span className="text-[var(--syntax-punct)]">:</span>}
      {editing === "value" ? (
        <InlineInput
          initial={row.node.t === "str" ? editableString(row.node.v) : row.node.t === "num" ? row.node.raw : ""}
          validate={row.node.t === "num" ? (t) => (isJsonNumber(t.trim()) ? null : "Not a JSON number") : undefined}
          onCommit={(t) => onCommit(row, "value", row.node.t === "str" ? fromEditable(row.node.v, t) : t)}
          onCancel={onCancel}
          testId="json-value-input"
        />
      ) : (
        <span className="flex min-w-0 items-center gap-1" onDoubleClick={() => onStartEdit(row, "value")}>
          <Value row={row} />
        </span>
      )}
      {replaced && (
        <span className="ml-1 rounded-sm bg-[var(--diff-change-bg)] px-1 font-sans text-[10px] text-[var(--diff-change)]" data-testid="json-replaced-badge">
          replaced as a whole
        </span>
      )}
      {row.removed > 0 && (
        <span className="ml-1 rounded-sm bg-[var(--diff-del-bg)] px-1 font-sans text-[10px] text-[var(--diff-del)]" title="Original members or items removed">
          −{row.removed}
        </span>
      )}
    </div>
  );
});

/** Strings with line breaks or tabs are edited escaped, so they fit one line. */
function editableString(v: string): string {
  return /[\n\r\t]/.test(v) ? JSON.stringify(v).slice(1, -1) : v;
}

function fromEditable(original: string, text: string): string {
  if (!/[\n\r\t]/.test(original)) return text;
  try {
    return JSON.parse(`"${text}"`) as string;
  } catch {
    return text;
  }
}

function InlineInput({
  initial,
  validate,
  onCommit,
  onCancel,
  testId,
}: {
  initial: string;
  validate?: (text: string) => string | null;
  onCommit: (text: string) => string | null;
  onCancel: () => void;
  testId: string;
}) {
  const [text, setText] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  // Enter and Escape finish the edit; the blur that follows mustn't commit again.
  const done = useRef(false);
  const invalid = error ?? validate?.(text) ?? null;
  return (
    <input
      autoFocus
      value={text}
      spellCheck={false}
      aria-invalid={invalid !== null}
      title={invalid ?? undefined}
      className="border-ring bg-background text-foreground h-[18px] min-w-24 flex-1 rounded-sm border px-1 font-mono text-[11.5px] outline-none aria-invalid:border-[var(--destructive)]"
      onFocus={(e) => e.target.select()}
      onChange={(e) => {
        setText(e.target.value);
        setError(null);
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          if (validate?.(text)) return;
          const problem = onCommit(text);
          done.current = problem === null;
          setError(problem);
        } else if (e.key === "Escape") {
          e.preventDefault();
          done.current = true;
          onCancel();
        }
      }}
      onBlur={() => {
        if (done.current) return;
        done.current = true;
        if (!validate?.(text) && !onCommit(text)) return;
        onCancel();
      }}
      data-testid={testId}
    />
  );
}

function Value({ row }: { row: Row }) {
  const n = row.node;
  switch (n.t) {
    case "obj":
    case "arr": {
      const [open, close] = n.t === "arr" ? ["[", "]"] : ["{", "}"];
      const size = children(n).length;
      if (size === 0) return <span className="text-[var(--syntax-punct)]">{open + close}</span>;
      const unit = n.t === "arr" ? "item" : "key";
      return (
        <>
          <span className="text-[var(--syntax-punct)]">{row.expanded ? open : `${open}…${close}`}</span>
          <span className="text-muted-foreground text-[10.5px]">
            {size} {unit}
            {size === 1 ? "" : "s"}
          </span>
        </>
      );
    }
    case "str":
      return <span className="truncate text-[var(--syntax-string)]">{JSON.stringify(n.v)}</span>;
    case "num":
      return <span className="text-[var(--syntax-number)]">{n.raw}</span>;
    case "bool":
      return <span className="text-[var(--syntax-literal)]">{String(n.v)}</span>;
    case "null":
      return <span className="text-[var(--syntax-literal)]">null</span>;
  }
}
