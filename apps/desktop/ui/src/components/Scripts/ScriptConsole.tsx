import { useQuery } from "@tanstack/react-query";
import { Eraser } from "lucide-react";
import { useEffect, useState } from "react";

import type { ConsoleEntry_Serialize, ConsoleLevel } from "@/bindings";
import { Checkbox } from "@/components/Rules/fields";
import { Button } from "@/components/ui/button";
import { backend } from "@/lib/backend";
import { cn } from "@/lib/utils";
import { startScriptConsole, useScriptConsole } from "@/store/scriptConsole";
import { useUi } from "@/store/ui";

const LEVEL_COLORS: Record<ConsoleLevel, string> = {
  debug: "text-muted-foreground",
  log: "text-foreground",
  info: "text-[var(--s-3xx)]",
  warn: "text-[var(--s-4xx)]",
  error: "text-[var(--s-failed)]",
};

/** What scripts logged (`console.*`) and the errors they hit, newest last. */
export function ScriptConsole({ scriptId }: { scriptId: string | null }) {
  useEffect(startScriptConsole, []);
  const latest = useScriptConsole((s) => s.latest);
  const [onlyThis, setOnlyThis] = useState(scriptId !== null);
  const { data: entries = [] } = useQuery({
    queryKey: ["script-console", latest],
    queryFn: () => backend.getScriptConsole(null, 2000),
    placeholderData: (previous) => previous,
  });
  const shown = onlyThis && scriptId ? entries.filter((e) => e.script_id === scriptId) : entries;
  return (
    <div className="flex h-full flex-col" data-testid="script-console">
      <div className="flex items-center gap-2 border-b px-2 py-1 text-xs">
        {scriptId && <Checkbox checked={onlyThis} onChange={setOnlyThis} label="This script only" />}
        <span className="text-muted-foreground">{shown.length} entries</span>
        <Button variant="ghost" size="xs" className="ml-auto" onClick={() => void backend.clearScriptConsole()} data-testid="clear-console">
          <Eraser />
          Clear
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto" data-selectable>
        {shown.length === 0 ? (
          <div className="text-muted-foreground p-2 text-xs">Nothing logged yet. Use console.log in a script.</div>
        ) : (
          shown.map((entry) => <ConsoleLine key={entry.seq} entry={entry} showScript={!onlyThis || !scriptId} />)
        )}
      </div>
    </div>
  );
}

export function ConsoleLine({ entry, showScript = false }: { entry: ConsoleEntry_Serialize; showScript?: boolean }) {
  const select = useUi((s) => s.select);
  const time = new Date(entry.time_ms ?? 0).toLocaleTimeString(undefined, { hour12: false });
  return (
    <div className="flex gap-2 border-b px-2 py-0.5 font-mono text-2xs leading-4 last:border-b-0" data-testid="console-line" data-level={entry.level}>
      <span className="text-muted-foreground shrink-0">{time}</span>
      {showScript && <span className="text-muted-foreground max-w-40 shrink-0 truncate">{entry.script_name}</span>}
      {entry.flow_id !== null && (
        <button type="button" className="text-muted-foreground shrink-0 hover:underline" title="Select this flow" onClick={() => select(entry.flow_id)}>
          #{entry.flow_id + 1}
        </button>
      )}
      <span className={cn("min-w-0 flex-1 break-all whitespace-pre-wrap", LEVEL_COLORS[entry.level])}>{entry.message}</span>
    </div>
  );
}
