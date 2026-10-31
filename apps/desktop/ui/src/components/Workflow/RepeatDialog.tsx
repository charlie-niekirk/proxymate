import { Repeat } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { RepeatProgress_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatMs, statusToken } from "@/lib/flow";
import { LIVE_STORE, useUi } from "@/store/ui";

/** Limits, as the engine checks them (`pm_core::MAX_REPEAT_*`). */
const MAX_ITERATIONS = 10_000;
const MAX_CONCURRENCY = 64;

/**
 * Repeat… (⌥⌘R): sends a recorded request N times, C at a time, each a new
 * live flow marked as repeated. Progress and the status counts and
 * latency percentiles update as it runs; closing the dialog cancels it.
 */
export function RepeatDialog() {
  const { open, store, flow } = useUi((s) => s.repeat);
  const close = useUi((s) => s.closeRepeat);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-[460px]" data-testid="repeat-dialog">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-sm">
            <Repeat className="size-4" />
            Repeat
          </DialogTitle>
          <DialogDescription>Each request is recorded as a new flow in the live capture.</DialogDescription>
        </DialogHeader>
        {open && flow !== null && <RepeatForm store={store} flow={flow} onClose={close} />}
      </DialogContent>
    </Dialog>
  );
}

function clamp(text: string, min: number, max: number): number {
  const n = Math.round(Number(text));
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : min;
}

function RepeatForm({ store, flow, onClose }: { store: number; flow: number; onClose: () => void }) {
  const [iterations, setIterations] = useState("100");
  const [concurrency, setConcurrency] = useState("4");
  const [delay, setDelay] = useState("0");
  const [applyRules, setApplyRules] = useState(true);
  const [progress, setProgress] = useState<RepeatProgress_Serialize | null>(null);
  const [error, setError] = useState<string | null>(null);
  const running = useRef<number | null>(null);
  const live = progress !== null && !progress.finished;

  // Closing the dialog stops a repeat that is still running.
  useEffect(
    () => () => {
      if (running.current !== null) void backend.cancelRepeat(running.current);
    },
    [],
  );

  const start = async () => {
    setError(null);
    setProgress(null);
    try {
      running.current = await backend.startRepeat(
        store,
        flow,
        {
          iterations: clamp(iterations, 1, MAX_ITERATIONS),
          concurrency: clamp(concurrency, 1, MAX_CONCURRENCY),
          delay_ms: clamp(delay, 0, 60_000),
          apply_rules: applyRules,
        },
        (p) => {
          setProgress(p);
          if (p.finished) running.current = null;
        },
      );
    } catch (e) {
      setError(errorText(e));
    }
  };

  const showFlows = () => {
    useUi.getState().setStore(LIVE_STORE);
    useUi.getState().setText("");
    if (progress?.last_flow != null) useUi.getState().select(progress.last_flow);
    onClose();
  };

  return (
    <form
      className="flex flex-col gap-3 text-xs"
      onSubmit={(e) => {
        e.preventDefault();
        if (!live) void start();
      }}
    >
      <div className="grid grid-cols-3 gap-2">
        <Field label="Iterations" hint={`1–${MAX_ITERATIONS.toLocaleString()}`}>
          <Input value={iterations} onChange={(e) => setIterations(e.target.value)} inputMode="numeric" className="h-7 text-xs" data-testid="repeat-iterations" disabled={live} />
        </Field>
        <Field label="Concurrency" hint={`1–${MAX_CONCURRENCY}`}>
          <Input value={concurrency} onChange={(e) => setConcurrency(e.target.value)} inputMode="numeric" className="h-7 text-xs" data-testid="repeat-concurrency" disabled={live} />
        </Field>
        <Field label="Delay (ms)" hint="before each">
          <Input value={delay} onChange={(e) => setDelay(e.target.value)} inputMode="numeric" className="h-7 text-xs" data-testid="repeat-delay" disabled={live} />
        </Field>
      </div>
      <label className="flex items-center gap-2">
        <Switch checked={applyRules} onCheckedChange={setApplyRules} disabled={live} data-testid="repeat-rules" />
        Apply rules (rewrites, Map Local, breakpoints, the throttle)
      </label>
      {progress && <Summary progress={progress} />}
      {error && <p className="text-destructive">{error}</p>}
      <DialogFooter>
        {progress?.finished && progress.last_flow !== null && (
          <Button type="button" variant="ghost" size="sm" className="mr-auto" onClick={showFlows} data-testid="repeat-show">
            Show Flows
          </Button>
        )}
        {live ? (
          <Button type="button" variant="outline" size="sm" onClick={() => void backend.cancelRepeat(progress.id)} data-testid="repeat-cancel">
            Stop
          </Button>
        ) : (
          <Button type="button" variant="outline" size="sm" onClick={onClose}>
            Close
          </Button>
        )}
        <Button type="submit" size="sm" disabled={live} data-testid="repeat-start">
          {progress?.finished ? "Run Again" : "Start"}
        </Button>
      </DialogFooter>
    </form>
  );
}

function Field({ label, hint, children }: { label: string; hint: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-muted-foreground">
        {label} <span className="opacity-70">({hint})</span>
      </span>
      {children}
    </label>
  );
}

function Summary({ progress: p }: { progress: RepeatProgress_Serialize }) {
  const pct = p.total === 0 ? 0 : Math.round((p.done / p.total) * 100);
  return (
    <div className="bg-muted/40 flex flex-col gap-2 rounded-md border p-2" data-testid="repeat-summary">
      <div className="flex items-center gap-2">
        <div className="bg-muted h-1.5 flex-1 overflow-hidden rounded-full">
          <div className="bg-primary h-full transition-[width]" style={{ width: `${pct}%` }} />
        </div>
        <span className="tabular-nums" data-testid="repeat-done">
          {p.done.toLocaleString()} / {p.total.toLocaleString()}
        </span>
      </div>
      <div className="flex flex-wrap gap-1.5" data-testid="repeat-statuses">
        {p.statuses.map((s) => {
          const token = s.status === null ? "failed" : statusToken(s.status, "completed");
          return (
            <span
              key={String(s.status)}
              className="rounded px-1.5 py-0.5 font-mono tabular-nums"
              style={{ color: `var(--s-${token})`, background: `var(--s-${token}-bg, transparent)` }}
            >
              {s.status ?? "failed"} × {s.count.toLocaleString()}
            </span>
          );
        })}
      </div>
      <div className="text-muted-foreground grid grid-cols-4 gap-1 tabular-nums">
        <span>min {formatMs(p.min_ms)}</span>
        <span>p50 {formatMs(p.p50_ms)}</span>
        <span>p95 {formatMs(p.p95_ms)}</span>
        <span>max {formatMs(p.max_ms)}</span>
      </div>
      {p.finished && (
        <div className="text-muted-foreground">
          {p.error ? <span className="text-destructive">{p.error}</span> : p.cancelled ? "Stopped." : `Done${p.failed ? `, ${p.failed} failed` : ""}.`}
        </div>
      )}
    </div>
  );
}
