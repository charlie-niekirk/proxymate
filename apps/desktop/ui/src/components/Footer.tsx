import { ArrowDownToLine, Filter, Trash2 } from "lucide-react";

import { SEARCH_INPUT_ID } from "@/components/FilterBar";
import { Button } from "@/components/ui/button";
import { Toggle } from "@/components/ui/toggle";
import { clearFlows } from "@/lib/actions";
import { useUi } from "@/store/ui";

const nf = new Intl.NumberFormat();

export function Footer() {
  const { len, total } = useUi((s) => s.stats);
  const selected = useUi((s) => s.selectedId !== null);
  const timing = useUi((s) => s.filterTiming);
  const autoScroll = useUi((s) => s.autoScroll);
  const setAutoScroll = useUi((s) => s.setAutoScroll);

  return (
    <footer className="bg-pane-toolbar text-muted-foreground flex h-7 shrink-0 items-center gap-1 border-t px-2 text-xs">
      <Button variant="ghost" size="sm" onClick={() => void clearFlows()}>
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
      {timing && (
        <span className="tabular-nums" data-testid="filter-timing" title="Rust refilter · filter change to painted rows">
          filter {timing.engineMs.toFixed(1)} ms · {timing.endToEndMs.toFixed(0)} ms end to end
        </span>
      )}
      <span className="ml-3 font-mono tabular-nums">↑ 0 B/s ↓ 0 B/s</span>
    </footer>
  );
}
