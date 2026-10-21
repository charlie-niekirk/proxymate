import type { FlowSummary_Serialize, Timing_Serialize } from "@/bindings";
import { Placeholder } from "@/components/Inspector/KvTable";
import { formatDuration, formatMs } from "@/lib/flow";
import { timingPhases } from "@/lib/tls";

/** The Timing tab: a small waterfall of connection and transfer phases. */
export function TimingView({ flow, timing }: { flow: FlowSummary_Serialize; timing: Timing_Serialize }) {
  const phases = timingPhases(timing);
  if (phases.length === 0) return <Placeholder text={flow.state === "active" ? "Waiting for the upstream…" : "No timings recorded"} />;
  const last = phases[phases.length - 1]!;
  const total = Math.max(last.offset + last.ms, 0.001);
  return (
    <div className="flex flex-col gap-1 p-2 text-xs" data-testid="timing-view">
      {phases.map((p) => {
        const left = (p.offset / total) * 100;
        const width = Math.max((p.ms / total) * 100, 0.5);
        return (
          <div key={p.label} className="grid grid-cols-[110px_1fr_72px] items-center gap-2">
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
      <div className="mt-1 grid grid-cols-[110px_1fr_72px] items-center gap-2 border-t pt-1.5">
        <span className="font-medium">Total</span>
        <span className="text-muted-foreground">
          {timing.reused_connection ? "Reused an open upstream connection" : ""}
        </span>
        <span className="text-right font-mono tabular-nums">{formatDuration(flow.duration_ms)}</span>
      </div>
    </div>
  );
}
