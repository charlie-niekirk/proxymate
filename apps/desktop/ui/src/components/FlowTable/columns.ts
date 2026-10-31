import type { HeaderColumn, SortKey } from "@/bindings";

export type BuiltinColumnId =
  | "id"
  | "state"
  | "url"
  | "client"
  | "method"
  | "status"
  | "time"
  | "duration"
  | "req"
  | "resp"
  | "proto"
  | "ssl"
  | "notes"
  | "tools";

/** A header column (M17): `hdr:request:x-request-id`. */
export type HeaderColumnId = `hdr:${"request" | "response"}:${string}`;

export type ColumnId = BuiltinColumnId | HeaderColumnId;

export interface Column {
  id: ColumnId;
  /** Header text; empty for icon-only columns. */
  title: string;
  /** What the column menu calls it. */
  label: string;
  /** Default width in px; the URL column grows past it. */
  width: number;
  minWidth: number;
  align?: "left" | "right" | "center";
  sortKey?: SortKey;
  /** Takes the leftover width. */
  grow?: boolean;
  /** Can't be hidden. */
  required?: boolean;
  /** A header column: which header, and its index in the rows' `extra`. */
  header?: HeaderColumn & { index: number };
}

/** Every column, in default order. Users resize, reorder and hide them. */
export const COLUMNS: readonly Column[] = [
  { id: "id", title: "#", label: "Number", width: 64, minWidth: 36, align: "right", sortKey: "id" },
  { id: "state", title: "", label: "State", width: 18, minWidth: 18, align: "center" },
  { id: "url", title: "URL", label: "URL", width: 240, minWidth: 120, sortKey: "url", grow: true, required: true },
  { id: "client", title: "Client", label: "Client", width: 120, minWidth: 48 },
  { id: "method", title: "Method", label: "Method", width: 72, minWidth: 48, align: "center", sortKey: "method" },
  { id: "status", title: "Status", label: "Status", width: 64, minWidth: 48, align: "center", sortKey: "status" },
  { id: "time", title: "Time", label: "Time", width: 92, minWidth: 56, align: "right" },
  { id: "duration", title: "Duration", label: "Duration", width: 72, minWidth: 48, align: "right", sortKey: "duration" },
  { id: "req", title: "Request", label: "Request size", width: 72, minWidth: 48, align: "right", sortKey: "req_size" },
  { id: "resp", title: "Response", label: "Response size", width: 76, minWidth: 48, align: "right", sortKey: "resp_size" },
  { id: "proto", title: "Proto", label: "Protocol", width: 42, minWidth: 32, align: "center" },
  { id: "ssl", title: "SSL", label: "SSL", width: 34, minWidth: 28, align: "center" },
  { id: "notes", title: "Notes", label: "Notes (label, comment, edited)", width: 54, minWidth: 36, align: "center" },
  { id: "tools", title: "", label: "Tools", width: 28, minWidth: 28, align: "center" },
];

export const COLUMN_BY_ID = Object.fromEntries(COLUMNS.map((c) => [c.id, c])) as Record<BuiltinColumnId, Column>;

export function headerColumnId(c: HeaderColumn): HeaderColumnId {
  return `hdr:${c.part}:${c.name.toLowerCase()}`;
}

export function isHeaderColumnId(id: string): id is HeaderColumnId {
  return id.startsWith("hdr:");
}

/** The column for header column number `index`. */
export function headerColumnDef(c: HeaderColumn, index: number): Column {
  return {
    id: headerColumnId(c),
    title: c.name,
    label: `${c.part === "request" ? "Request" : "Response"} header ${c.name}`,
    width: 140,
    minWidth: 48,
    sortKey: "extra",
    header: { ...c, index },
  };
}

/** Builtin columns plus the header columns, by id. */
export function columnsById(headers: readonly HeaderColumn[]): Map<ColumnId, Column> {
  const map = new Map<ColumnId, Column>(COLUMNS.map((c) => [c.id, c]));
  headers.forEach((h, i) => map.set(headerColumnId(h), headerColumnDef(h, i)));
  return map;
}

export const ROW_HEIGHT = 22;

/** A visible column with its current width. */
export interface PlacedColumn extends Column {
  px: number;
}

/** The columns to draw, in order, from a saved layout and the header columns. */
export function placeColumns(
  layout: {
    order: readonly ColumnId[];
    widths: Partial<Record<ColumnId, number>>;
    hidden: readonly ColumnId[];
  },
  headers: readonly HeaderColumn[] = [],
): PlacedColumn[] {
  const byId = columnsById(headers);
  return layout.order.flatMap((id) => {
    const col = byId.get(id);
    if (!col || (!col.required && layout.hidden.includes(id))) return [];
    return [{ ...col, px: Math.max(col.minWidth, layout.widths[id] ?? col.width) }];
  });
}

/** The CSS grid track list for `columns`. */
export function gridTemplate(columns: readonly PlacedColumn[]): string {
  return columns.map((c) => (c.grow ? `minmax(${c.px}px, 1fr)` : `${c.px}px`)).join(" ");
}

/** Sum of the columns' widths: the table scrolls sideways below it. */
export function minTableWidth(columns: readonly PlacedColumn[]): number {
  return columns.reduce((n, c) => n + c.px, 0);
}
