import type { SortKey } from "@/bindings";

export interface Column {
  id: string;
  title: string;
  /** CSS grid track size. */
  width: string;
  align?: "left" | "right" | "center";
  sortKey?: SortKey;
}

/** Fixed column set for M0; resize, reorder, hide and persistence land in M10. */
export const COLUMNS: readonly Column[] = [
  { id: "id", title: "#", width: "64px", align: "right", sortKey: "id" },
  { id: "state", title: "", width: "18px", align: "center" },
  { id: "url", title: "URL", width: "minmax(240px, 1fr)", sortKey: "url" },
  { id: "client", title: "Client", width: "120px" },
  { id: "method", title: "Method", width: "72px", align: "center", sortKey: "method" },
  { id: "status", title: "Status", width: "64px", align: "center", sortKey: "status" },
  { id: "time", title: "Time", width: "92px", align: "right" },
  { id: "duration", title: "Duration", width: "72px", align: "right", sortKey: "duration" },
  { id: "req", title: "Request", width: "72px", align: "right", sortKey: "req_size" },
  { id: "resp", title: "Response", width: "76px", align: "right", sortKey: "resp_size" },
  { id: "proto", title: "Proto", width: "42px", align: "center" },
  { id: "ssl", title: "SSL", width: "34px", align: "center" },
  { id: "edited", title: "Edited", width: "46px", align: "center" },
  { id: "tools", title: "", width: "28px", align: "center" },
];

export const GRID_TEMPLATE = COLUMNS.map((c) => c.width).join(" ");

export const ROW_HEIGHT = 22;
