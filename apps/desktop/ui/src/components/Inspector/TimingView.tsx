import type { ReactElement } from "react";

import type { FlowSummary_Serialize, Timing_Serialize, Via_Serialize } from "@/bindings";
import { Placeholder } from "@/components/Inspector/KvTable";
import { CopyItem } from "@/components/menus/items";
import { useMenuTarget } from "@/components/menus/useMenuTarget";
import { ContextMenu, ContextMenuContent, ContextMenuTrigger } from "@/components/ui/context-menu";
import { tableText } from "@/lib/copyFormats";
import { formatDuration, formatMs } from "@/lib/flow";
import { routeRows, timingPhases } from "@/lib/tls";

/** The Timing tab: a small waterfall of connection and transfer phases, and the route the connection took. */
export function TimingView({
  flow,
  timing,
  via = null,
}: {
  flow: FlowSummary_Serialize;
  timing: Timing_Serialize;
  via?: Via_Serialize | null;
}) {
  const phases = timingPhases(timing);
  if (phases.length === 0) return <Placeholder text={flow.state === "active" ? "Waiting for the upstream…" : "No timings recorded"} />;
  const last = phases[phases.length - 1]!;
  const total = Math.max(last.offset + last.ms, 0.001);
  const route = routeRows(via);
  const table: [string, string][] = [
    ...phases.map((p): [string, string] => [p.label, formatMs(p.ms)]),
    ["Total", formatDuration(flow.duration_ms)],
    ...route,
  ];
  return (
    <TimingMenu table={table}>
      <div className="flex flex-col gap-1 p-2 text-xs" data-testid="timing-view">
        {phases.map((p, i) => {
          const left = (p.offset / total) * 100;
          const width = Math.max((p.ms / total) * 100, 0.5);
          return (
            <div key={p.label} className="grid grid-cols-[110px_1fr_72px] items-center gap-2" data-menu-key={i}>
              <span className="text-muted-foreground">{p.label}</span>
              <div className="bg-muted relative h-2.5 rounded-sm">
                <div
                  className="absolute inset-y-0 rounded-sm"
                  style={{ left: `${left}%`, width: `${width}%`, backgroundColor: `var(${p.token})` }}
                />
              </div>
              <span className="text-right font-mono tabular-nums">{formatMs(p.ms)}</span>
            </div>
          );
        })}
        <div className="mt-1 grid grid-cols-[110px_1fr_72px] items-center gap-2 border-t pt-1.5" data-menu-key={phases.length}>
          <span className="font-medium">Total</span>
          <span className="text-muted-foreground">
            {timing.reused_connection ? "Reused an open upstream connection" : ""}
          </span>
          <span className="text-right font-mono tabular-nums">{formatDuration(flow.duration_ms)}</span>
        </div>
        {route.length > 0 && (
          <div className="mt-1 flex flex-col gap-0.5 border-t pt-1.5" data-testid="timing-route">
            {route.map(([label, value], i) => (
              <div key={label} className="grid grid-cols-[110px_1fr] gap-2" data-menu-key={phases.length + 1 + i}>
                <span className="text-muted-foreground">{label}</span>
                <span className="min-w-0 truncate font-mono text-[11px]" title={value}>
                  {value}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </TimingMenu>
  );
}

/** Copy a phase's time, or the whole table (tab-separated, for a spreadsheet). */
function TimingMenu({ table, children }: { table: [string, string][]; children: ReactElement }) {
  const menu = useMenuTarget((key) => table[Number(key)], { always: true });
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={menu.onContextMenu}>
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-44" data-testid="timing-menu">
        {menu.target && (
          <CopyItem text={menu.target[1]} what={menu.target[0]}>
            Copy {menu.target[0]}
          </CopyItem>
        )}
        <CopyItem text={tableText(table)} what="timing" testId="timing-copy-table">
          Copy timing as table
        </CopyItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
