import { useQuery } from "@tanstack/react-query";
import { Check, Gauge } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { setThrottle } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { rulesKey } from "@/lib/query";
import { GLOBAL_THROTTLE_RULE_ID, THROTTLE_PRESETS, activeThrottle, normalizeRule, type ThrottlePreset } from "@/lib/rules";
import { cn } from "@/lib/utils";

const kbps = (n: number) => (n >= 1000 ? `${n / 1000} Mbps` : `${n} kbps`);

/**
 * Network conditions for all traffic: the built-in throttle rule, which
 * matches everything and is off until a profile is picked here.
 */
export function ThrottleMenu() {
  const { data: rules } = useQuery({ queryKey: rulesKey, queryFn: () => backend.listRules() });
  const rule = rules?.find((r) => r.id === GLOBAL_THROTTLE_RULE_ID);
  const active = activeThrottle(rule ? normalizeRule(rule) : undefined);
  const [open, setOpen] = useState(false);
  const pick = (preset: ThrottlePreset | null) => {
    void setThrottle(preset);
    setOpen(false);
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size={active ? "sm" : "icon"}
          aria-label="Network conditions"
          aria-pressed={!!active}
          data-testid="throttle-menu"
          className={cn(active ? "text-[var(--s-4xx)]" : "text-muted-foreground")}
        >
          <Gauge />
          {active && <span className="text-xs">{active.name}</span>}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 p-1 text-xs">
        <div className="text-muted-foreground px-2 py-1">Throttle all traffic</div>
        <Item label="Off" detail="Full speed" checked={!active} onClick={() => pick(null)} />
        {THROTTLE_PRESETS.map((p) => (
          <Item
            key={p.name}
            label={p.name}
            detail={`${kbps(p.download_kbps)} ↓ · ${kbps(p.upload_kbps)} ↑ · ${p.latency_ms} ms`}
            checked={active?.name === p.name}
            onClick={() => pick(p)}
          />
        ))}
        {active?.name === "Custom" && <Item label="Custom" detail="Edited in the Rules Editor" checked onClick={() => setOpen(false)} />}
      </PopoverContent>
    </Popover>
  );
}

function Item({ label, detail, checked, onClick }: { label: string; detail: string; checked: boolean; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} className="hover:bg-accent flex w-full items-center gap-2 rounded-sm px-2 py-1 text-left">
      <Check className={cn("size-3", !checked && "invisible")} />
      <span className="flex-1">{label}</span>
      <span className="text-muted-foreground font-mono text-2xs">{detail}</span>
    </button>
  );
}
