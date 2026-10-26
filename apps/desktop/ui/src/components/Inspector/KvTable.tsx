import type { ReactNode } from "react";

import { CopyItem, SelectionItem } from "@/components/menus/items";
import { useMenuTarget } from "@/components/menus/useMenuTarget";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { addFilterTerm } from "@/lib/actions";
import {
  authToken,
  dslTerm,
  formPair,
  headerLine,
  headersBlock,
  pairsJson,
  parseCookie,
  parseSetCookie,
  plainBlock,
  queryString,
  type Cookie,
} from "@/lib/copyFormats";


export type Kv = [string, string][];

/**
 * What the rows are, for what their menu copies: header lines, URL-encoded
 * query or form pairs, or plain `name: value` rows (TLS, certificates).
 */
export type KvKind = "header" | "query" | "form" | "plain";

export function KvTable({
  rows,
  kind = "plain",
  side,
  testId,
}: {
  rows: Kv;
  kind?: KvKind;
  /** For headers: which message, so "Filter flows" searches that side. */
  side?: "request" | "response";
  testId?: string;
}) {
  const menu = useMenuTarget((key) => (rows[Number(key)] ? Number(key) : null));
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <table className="w-full font-mono text-xs" data-selectable data-testid={testId} onContextMenu={menu.onContextMenu}>
          <tbody>
            {rows.map(([k, v], i) => (
              <tr key={`${k}-${i}`} className="border-border/50 border-b" data-menu-key={i}>
                <td className="w-1/3 px-2 py-1 align-top font-medium text-[var(--syntax-key)]">{k}</td>
                <td className="px-2 py-1 break-all">{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </ContextMenuTrigger>
      <KvMenu rows={rows} kind={kind} side={side} index={menu.target} selection={menu.selection} />
    </ContextMenu>
  );
}

/**
 * The menu of a name/value table: the row's name, value and line (plus
 * cookies, tokens and a filter for headers), then the whole table. Shared
 * with tables laid out differently (form fields, multipart part headers).
 */
export function KvMenu({
  rows,
  kind,
  side,
  index,
  selection,
  children,
}: {
  rows: Kv;
  kind: KvKind;
  side?: "request" | "response";
  /** The row right-clicked, if any. */
  index: number | null;
  selection: string;
  /** Items after the table's (a body viewer's body items). */
  children?: ReactNode;
}) {
  const row = index === null ? undefined : rows[index];
  return (
    <ContextMenuContent className="min-w-48" data-testid="kv-menu">
      <SelectionItem text={selection} />
      {row && (
        <>
          <RowItems row={row} kind={kind} side={side} />
          <ContextMenuSeparator />
        </>
      )}
      <CopyItem text={allText(rows, kind)} what={ALL_LABEL[kind]} testId="kv-copy-all">
        Copy all {ALL_LABEL[kind]}
      </CopyItem>
      <CopyItem text={pairsJson(rows)} what={`${ALL_LABEL[kind]} as JSON`}>
        Copy all as JSON
      </CopyItem>
      {children}
    </ContextMenuContent>
  );
}

const ALL_LABEL: Record<KvKind, string> = { header: "headers", query: "parameters", form: "fields", plain: "rows" };

function allText(rows: Kv, kind: KvKind): string {
  switch (kind) {
    case "header":
      return headersBlock(rows);
    case "query":
    case "form":
      return queryString(rows);
    case "plain":
      return plainBlock(rows);
  }
}

function RowItems({ row: [name, value], kind, side }: { row: [string, string]; kind: KvKind; side?: "request" | "response" }) {
  const header = kind === "header";
  const lower = name.toLowerCase();
  const cookies: Cookie[] = !header
    ? []
    : lower === "cookie"
      ? parseCookie(value)
      : lower === "set-cookie"
        ? [parseSetCookie(value)].filter((c): c is Cookie => c !== null)
        : [];
  const token = header && (lower === "authorization" || lower === "proxy-authorization") ? authToken(value) : null;
  const filter = () => addFilterTerm(dslTerm(side === "request" ? "reqheader" : "resheader", headerLine(name, value)));
  return (
    <>
      <CopyItem text={name} what={header ? "header name" : "name"} testId="kv-copy-name">
        Copy name
      </CopyItem>
      <CopyItem text={value} what={header ? "header value" : "value"} testId="kv-copy-value">
        Copy value
      </CopyItem>
      {header ? (
        <CopyItem text={headerLine(name, value)} what="header" testId="kv-copy-line">
          Copy as “Name: value”
        </CopyItem>
      ) : kind === "plain" ? (
        <CopyItem text={`${name}: ${value}`} what="row" testId="kv-copy-line">
          Copy row
        </CopyItem>
      ) : (
        <CopyItem text={formPair(name, value)} what={kind === "query" ? "parameter" : "field"} testId="kv-copy-line">
          Copy as name=value
        </CopyItem>
      )}
      {token !== null && (
        <CopyItem text={token} what="token" testId="kv-copy-token">
          Copy token
        </CopyItem>
      )}
      {cookies.length === 1 && (
        <CopyItem text={cookies[0]!.value} what={`cookie ${cookies[0]!.name}`} testId="kv-copy-cookie">
          Copy cookie value
        </CopyItem>
      )}
      {cookies.length > 1 && (
        <ContextMenuSub>
          <ContextMenuSubTrigger data-testid="kv-cookies">Copy cookie</ContextMenuSubTrigger>
          <ContextMenuSubContent className="max-h-80 max-w-80 overflow-y-auto">
            {cookies.map((c, i) => (
              <CopyItem key={`${c.name}-${i}`} text={c.value} what={`cookie ${c.name}`} testId={`kv-cookie-${c.name}`}>
                <span className="truncate">{c.name}</span>
              </CopyItem>
            ))}
          </ContextMenuSubContent>
        </ContextMenuSub>
      )}
      {header && side && (
        <>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={filter} data-testid="kv-filter">
            Filter flows with this header
          </ContextMenuItem>
        </>
      )}
    </>
  );
}

export function Placeholder({ text }: { text: string }) {
  return <div className="text-muted-foreground p-4 text-center text-xs">{text}</div>;
}
