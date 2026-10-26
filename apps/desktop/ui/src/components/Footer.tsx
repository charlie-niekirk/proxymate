import { ArrowDownToLine, CircleAlert, CircleCheck, Filter, HardDriveDownload, LoaderCircle, ScanSearch, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { SEARCH_INPUT_ID } from "@/components/FilterBar";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { ContextMenuItem } from "@/components/ui/context-menu";
import { Toggle } from "@/components/ui/toggle";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { clearFlows } from "@/lib/actions";
import { shortcutLabel } from "@/lib/commands";
import { formatSize } from "@/lib/flow";
import { useStorage } from "@/store/storage";
import { LIVE_STORE, useUi, type SaveJob } from "@/store/ui";

const nf = new Intl.NumberFormat();

export function Footer() {
  const { len, total } = useUi((s) => s.stats);
  const selected = useUi((s) => s.selectedId !== null);
  const timing = useUi((s) => s.filterTiming);
  const autoScroll = useUi((s) => s.autoScroll);
  const setAutoScroll = useUi((s) => s.setAutoScroll);
  const live = useUi((s) => s.store === LIVE_STORE);
  const save = useUi((s) => s.save);
  const deep = useUi((s) => s.deep);

  return (
    <footer className="bg-pane-toolbar text-muted-foreground flex h-7 shrink-0 items-center gap-1 border-t px-2 text-xs">
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="sm" disabled={!live} onClick={() => void clearFlows()}>
            <Trash2 /> Clear
          </Button>
        </TooltipTrigger>
        <TooltipContent>Clear all flows · {shortcutLabel("Backspace")}</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="sm" onClick={() => document.getElementById(SEARCH_INPUT_ID)?.focus()}>
            <Filter /> Filter
          </Button>
        </TooltipTrigger>
        <TooltipContent>Search flows · {shortcutLabel("f")}</TooltipContent>
      </Tooltip>
      <Toggle size="sm" pressed={autoScroll} onPressedChange={setAutoScroll} className="gap-1">
        <ArrowDownToLine /> Auto Select
      </Toggle>
      <div className="flex-1 text-center tabular-nums" data-testid="row-count">
        {selected ? "1 / " : ""}
        {nf.format(len)} of {nf.format(total)} rows
      </div>
      {deep && <DeepStatus checked={deep.checked} total={deep.total} matched={deep.matched} running={deep.running} />}
      {save && <SaveStatus job={save} />}
      {live && <StorageWarning />}
      {timing && (
        <span className="tabular-nums" data-testid="filter-timing" title="Rust refilter · filter change to painted rows">
          filter {timing.engineMs.toFixed(1)} ms · {timing.endToEndMs.toFixed(0)} ms end to end
        </span>
      )}
      <Throughput />
    </footer>
  );
}

/** The table's deep search: progress while it runs, then what it found. */
function DeepStatus({ checked, total, matched, running }: { checked: number; total: number; matched: number; running: boolean }) {
  const pct = total > 0 ? Math.floor((checked / total) * 100) : 100;
  return (
    <span className="mr-3 flex items-center gap-1 tabular-nums" data-testid="deep-status">
      {running ? <LoaderCircle className="size-3 animate-spin" /> : <ScanSearch className="size-3" />}
      {running
        ? `Searching bodies · ${nf.format(checked)} of ${nf.format(total)} (${pct}%) · ${nf.format(matched)} found`
        : `Searched ${nf.format(total)} flows · ${nf.format(matched)} found`}
    </span>
  );
}

/** A failed write to the capture files (a full disk): bodies aren't being kept. */
function StorageWarning() {
  const error = useStorage((s) => s.status?.error ?? null);
  const openSettings = useUi((s) => s.openSettings);
  if (!error) return null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          className="mr-3 flex items-center gap-1 text-[var(--s-failed)]"
          onClick={() => openSettings("storage")}
          data-testid="storage-error"
        >
          <HardDriveDownload className="size-3" />
          Not recording bodies
        </button>
      </TooltipTrigger>
      <TooltipContent className="max-w-80">Proxymate {error}. Traffic is still forwarded. Click for Storage settings.</TooltipContent>
    </Tooltip>
  );
}

/** ↑ and ↓ bytes per second, averaged over the last second or so. */
function Throughput() {
  const status = useStorage((s) => s.status);
  const last = useRef<{ t: number; sent: number; received: number } | null>(null);
  const [rate, setRate] = useState({ up: 0, down: 0 });
  useEffect(() => {
    const id = setInterval(() => {
      const s = useStorage.getState().status;
      if (!s) return;
      const now = performance.now();
      const prev = last.current;
      last.current = { t: now, sent: s.sent_bytes, received: s.received_bytes };
      if (!prev) return;
      const secs = (now - prev.t) / 1000;
      // A clear or a new engine counts from zero: no negative rates.
      setRate({
        up: Math.max(0, (s.sent_bytes - prev.sent) / secs),
        down: Math.max(0, (s.received_bytes - prev.received) / secs),
      });
    }, 1000);
    return () => clearInterval(id);
  }, []);
  if (!status) return <span className="ml-3 font-mono tabular-nums">↑ — ↓ —</span>;
  const fmt = (n: number) => (n < 1 ? "0 B/s" : `${formatSize(Math.round(n))}/s`);
  return (
    <span className="ml-3 font-mono tabular-nums" data-testid="throughput" title="Body bytes through the proxy per second">
      ↑ {fmt(rate.up)} ↓ {fmt(rate.down)}
    </span>
  );
}

/** A background save or export: progress, then where it went or why it failed. */
function SaveStatus({ job }: { job: SaveJob }) {
  const { done, total, finished, error, path } = job.progress;
  const clear = useUi((s) => s.setSave);
  const name = path.split("/").pop() ?? path;
  if (error) {
    return (
      <CopyMenu
        items={[{ label: "Copy error", text: error, what: "error" }]}
        extra={<ContextMenuItem onSelect={() => clear(null)}>Dismiss</ContextMenuItem>}
      >
        <button className="mr-3 flex items-center gap-1 text-[var(--s-failed)]" onClick={() => clear(null)} title={error} data-testid="save-status">
          <CircleAlert className="size-3" />
          {job.label} failed: <span className="max-w-72 truncate">{error}</span>
        </button>
      </CopyMenu>
    );
  }
  if (finished) {
    return (
      <CopyMenu
        items={[
          { label: "Copy path", text: path, what: "path" },
          { label: "Copy file name", text: name, what: "file name" },
        ]}
        extra={<ContextMenuItem onSelect={() => clear(null)}>Dismiss</ContextMenuItem>}
      >
        <span className="mr-3 flex items-center gap-1" title={path} data-testid="save-status">
          <CircleCheck className="size-3 text-[var(--s-2xx)]" />
          Saved {nf.format(done)} flows to <span className="max-w-60 truncate">{name}</span>
        </span>
      </CopyMenu>
    );
  }
  const pct = total > 0 ? Math.floor((done / total) * 100) : 0;
  return (
    <span className="mr-3 flex items-center gap-1 tabular-nums" data-testid="save-status">
      <LoaderCircle className="size-3 animate-spin" />
      {job.label} · {pct}%
    </span>
  );
}
