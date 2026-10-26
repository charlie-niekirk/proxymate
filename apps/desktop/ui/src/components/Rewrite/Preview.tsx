import { useVirtualizer } from "@tanstack/react-virtual";
import { CircleCheck, CircleX, LoaderCircle } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";

import type { Header, Phase, RuleTestResult_Serialize } from "@/bindings";
import { CopyItem, CopyMenu, SelectionItem } from "@/components/menus/items";
import { useMenuTarget } from "@/components/menus/useMenuTarget";
import { ContextMenu, ContextMenuContent, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import { headerLine } from "@/lib/copyFormats";
import { displayHeaderName } from "@/lib/http";
import { lineDiff, prettyForDiff, type DiffLine } from "@/lib/rewrite/lines";
import { cn } from "@/lib/utils";

/**
 * The live dry run of the rule being built, on the side being edited: the
 * status, the headers as a key/value diff and the body as a line diff (the
 * engine keeps every byte it doesn't change, so changed lines are the
 * changed paths).
 */
export function Preview({
  side,
  result,
  busy,
  error,
  before,
}: {
  side: Phase;
  result: RuleTestResult_Serialize | null;
  busy: boolean;
  error: string | null;
  /** The recorded message on this side. */
  before: { status: number | null; headers: Header[]; body: string };
}) {
  const head = side === "request" ? result?.request : result?.response;
  const body = side === "request" ? result?.request_body : result?.response_body;
  const truncated = side === "request" ? result?.request_body_truncated : result?.response_body_truncated;
  const lines = useMemo(
    () => (body === null || body === undefined ? null : lineDiff(prettyForDiff(before.body), prettyForDiff(body))),
    [before.body, body],
  );
  const changedLines = lines?.filter((l) => l.kind !== "same").length ?? 0;
  return (
    <div className="flex h-full min-h-0 flex-col text-xs" data-testid="rewrite-preview">
      <div className="flex h-7 shrink-0 items-center gap-1.5 border-b px-2">
        {busy ? (
          <LoaderCircle className="text-muted-foreground size-3.5 animate-spin" />
        ) : result?.matched ? (
          <CircleCheck className="size-3.5 text-[var(--s-2xx)]" />
        ) : (
          <CircleX className="size-3.5 text-[var(--s-failed)]" />
        )}
        <span className="truncate" data-testid="preview-status">
          {error ? error : !result ? "Dry run…" : result.matched ? `The ${side} after the rule` : `No match: ${result.mismatch ?? ""}`}
        </span>
        {side === "response" && head && "status" in head && (
          <span
            className={cn("ml-auto rounded-sm px-1 font-mono", head.status !== before.status && "bg-[var(--diff-change-bg)] text-[var(--diff-change)]")}
            data-testid="preview-status-code"
          >
            {head.status}
          </span>
        )}
      </div>
      {result && result.notes.length > 0 && (
        <ul className="text-muted-foreground border-b px-5 py-1 list-disc" data-testid="preview-notes">
          {result.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
      {head && <HeaderDiff before={before.headers} after={head.headers} />}
      {truncated && (
        <div className="border-b bg-[var(--s-4xx-bg)] px-2 py-1" data-testid="preview-truncated">
          The body is over 2 MB: the preview shows its start.
        </div>
      )}
      <div className="text-muted-foreground flex h-5 shrink-0 items-center border-b bg-[var(--pane-header)] px-2 text-[10.5px] font-medium tracking-wide uppercase">
        Body
        {lines && <span className="ml-auto normal-case">{changedLines === 0 ? "unchanged" : `${changedLines} changed line${changedLines === 1 ? "" : "s"}`}</span>}
      </div>
      <CopyMenu
        items={[
          body && { label: `Copy ${side} body after`, text: body, what: "body" },
          before.body && { label: `Copy ${side} body before`, text: before.body, what: "body" },
        ]}
      >
        <div className="min-h-0 flex-1">{lines ? <Lines lines={lines} /> : <div className="text-muted-foreground p-2">No body.</div>}</div>
      </CopyMenu>
    </div>
  );
}

function HeaderDiff({ before, after }: { before: Header[]; after: Header[] }) {
  const key = (h: Header) => `${h.name.toLowerCase()}\n${h.value}`;
  const was = new Set(before.map(key));
  const now = new Set(after.map(key));
  const gone = before.filter((h) => !now.has(key(h)));
  const menu = useMenuTarget((k) => (k.startsWith("gone-") ? gone[Number(k.slice(5))] : after[Number(k)]), { always: true });
  const line = (h: Header) => headerLine(displayHeaderName(h.name), h.value);
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={menu.onContextMenu}>
        <div className="max-h-40 shrink-0 overflow-y-auto border-b py-0.5 font-mono text-[11px]" data-selectable data-testid="preview-headers">
          {after.map((h, i) => (
            <div key={i} data-changed={!was.has(key(h))} data-menu-key={i} className="flex gap-2 px-2 data-[changed=true]:bg-[var(--diff-add-bg)]">
              <span className="text-muted-foreground shrink-0">{displayHeaderName(h.name)}</span>
              <span className="truncate">{h.value}</span>
            </div>
          ))}
          {gone.map((h, i) => (
            <div key={`gone-${i}`} data-menu-key={`gone-${i}`} className="flex gap-2 bg-[var(--diff-del-bg)] px-2 text-[var(--diff-del)] line-through">
              <span className="shrink-0">{displayHeaderName(h.name)}</span>
              <span className="truncate">{h.value}</span>
            </div>
          ))}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-48" data-testid="preview-headers-menu">
        <SelectionItem text={menu.selection} />
        {menu.target && (
          <>
            <CopyItem text={line(menu.target)} what="header">
              Copy as “Name: value”
            </CopyItem>
            <CopyItem text={menu.target.value} what="header value">
              Copy value
            </CopyItem>
            <ContextMenuSeparator />
          </>
        )}
        <CopyItem text={after.map(line).join("\n")} what="headers">
          Copy headers after
        </CopyItem>
        <CopyItem text={before.map(line).join("\n")} what="headers">
          Copy headers before
        </CopyItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

const LINE = 18;

function Lines({ lines }: { lines: DiffLine[] }) {
  const scroller = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual is not compiler-safe; rows re-render through their props.
  const virtualizer = useVirtualizer({ count: lines.length, getScrollElement: () => scroller.current, estimateSize: () => LINE, overscan: 30 });
  // The first change in view, with some context above it.
  const first = lines.findIndex((l) => l.kind !== "same");
  useEffect(() => {
    if (first >= 0) virtualizer.scrollToIndex(Math.max(0, first - 3), { align: "start" });
  }, [first, virtualizer]);
  return (
    <div ref={scroller} className="h-full overflow-auto font-mono text-[11px]" data-testid="preview-body" data-selectable>
      <div style={{ height: virtualizer.getTotalSize(), position: "relative", minWidth: "100%" }}>
        {virtualizer.getVirtualItems().map((item) => {
          const line = lines[item.index]!;
          return (
            <div
              key={item.key}
              data-kind={line.kind}
              className={cn(
                "absolute left-0 flex w-full items-center whitespace-pre",
                "data-[kind=added]:bg-[var(--diff-add-bg)] data-[kind=removed]:bg-[var(--diff-del-bg)] data-[kind=removed]:text-[var(--diff-del)]",
              )}
              style={{ top: item.start, height: LINE }}
              data-testid="preview-line"
            >
              <span className="text-muted-foreground w-4 shrink-0 text-center">{line.kind === "added" ? "+" : line.kind === "removed" ? "−" : ""}</span>
              {line.text}
            </div>
          );
        })}
      </div>
    </div>
  );
}
