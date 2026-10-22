import { ArrowDownToLine, CircleAlert, CircleCheck, Filter, LoaderCircle, Trash2 } from "lucide-react";

import { SEARCH_INPUT_ID } from "@/components/FilterBar";
import { Button } from "@/components/ui/button";
import { Toggle } from "@/components/ui/toggle";
import { clearFlows } from "@/lib/actions";
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

  return (
    <footer className="bg-pane-toolbar text-muted-foreground flex h-7 shrink-0 items-center gap-1 border-t px-2 text-xs">
      <Button variant="ghost" size="sm" disabled={!live} onClick={() => void clearFlows()}>
        <Trash2 /> Clear
      </Button>
      <Button variant="ghost" size="sm" onClick={() => document.getElementById(SEARCH_INPUT_ID)?.focus()}>
        <Filter /> Filter
      </Button>
      <Toggle size="sm" pressed={autoScroll} onPressedChange={setAutoScroll} className="gap-1">
        <ArrowDownToLine /> Auto Select
      </Toggle>
      <div className="flex-1 text-center tabular-nums" data-testid="row-count">
        {selected ? "1 / " : ""}
        {nf.format(len)} of {nf.format(total)} rows
      </div>
      {save && <SaveStatus job={save} />}
      {timing && (
        <span className="tabular-nums" data-testid="filter-timing" title="Rust refilter · filter change to painted rows">
          filter {timing.engineMs.toFixed(1)} ms · {timing.endToEndMs.toFixed(0)} ms end to end
        </span>
      )}
      <span className="ml-3 font-mono tabular-nums">↑ 0 B/s ↓ 0 B/s</span>
    </footer>
  );
}

/** A background save or export: progress, then where it went or why it failed. */
function SaveStatus({ job }: { job: SaveJob }) {
  const { done, total, finished, error, path } = job.progress;
  const clear = useUi((s) => s.setSave);
  const name = path.split("/").pop() ?? path;
  if (error) {
    return (
      <button className="mr-3 flex items-center gap-1 text-[var(--s-failed)]" onClick={() => clear(null)} title={error} data-testid="save-status">
        <CircleAlert className="size-3" />
        {job.label} failed: <span className="max-w-72 truncate">{error}</span>
      </button>
    );
  }
  if (finished) {
    return (
      <span className="mr-3 flex items-center gap-1" title={path} data-testid="save-status">
        <CircleCheck className="size-3 text-[var(--s-2xx)]" />
        Saved {nf.format(done)} flows to <span className="max-w-60 truncate">{name}</span>
      </span>
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
