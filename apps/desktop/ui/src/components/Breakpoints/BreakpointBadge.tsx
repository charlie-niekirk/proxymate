import { OctagonPause } from "lucide-react";
import { useEffect } from "react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { startBreakpoints, useBreakpoints } from "@/store/breakpoints";
import { useUi } from "@/store/ui";

/** The toolbar's breakpoint queue: how many flows are paused. */
export function BreakpointBadge() {
  const count = useBreakpoints((s) => s.queue.length);
  const open = useUi((s) => s.setBreakpointsOpen);
  useEffect(startBreakpoints, []);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size={count ? "sm" : "icon"}
          aria-label="Breakpoints"
          onClick={() => open(true)}
          data-testid="breakpoint-badge"
          className={count ? "text-[var(--s-paused)]" : "text-muted-foreground"}
        >
          <OctagonPause />
          {count > 0 && (
            <span
              className="rounded-full px-1.5 font-mono text-2xs font-semibold"
              style={{ color: "var(--s-paused)", backgroundColor: "var(--s-paused-bg)" }}
            >
              {count}
            </span>
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{count ? `${count} paused at a breakpoint` : "Breakpoints · none paused"}</TooltipContent>
    </Tooltip>
  );
}
