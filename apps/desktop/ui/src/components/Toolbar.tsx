import {
  Bot,
  Circle,
  Gauge,
  Globe,
  OctagonPause,
  Plus,
  Settings,
  ShieldCheck,
  Trash2,
  type LucideIcon,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

function ToolButton({ icon: Icon, label, milestone }: { icon: LucideIcon; label: string; milestone: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" aria-label={label} className="text-muted-foreground">
          <Icon />
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {label} · arrives in {milestone}
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * The toolbar, drawn in the overlay title bar area. Empty space drags the
 * window. Controls are placeholders until their milestones land.
 */
export function Toolbar() {
  return (
    <header
      data-tauri-drag-region
      className="bg-pane-toolbar flex h-[52px] shrink-0 items-center gap-1 border-b px-2"
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="Record">
            <Circle className="fill-[var(--m-delete)] text-[var(--m-delete)]" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Recording · proxy arrives in M1</TooltipContent>
      </Tooltip>
      <ToolButton icon={Plus} label="Compose" milestone="M8" />
      <ToolButton icon={Trash2} label="Clear" milestone="M1" />

      <div data-tauri-drag-region className="flex flex-1 justify-center">
        <button className="text-muted-foreground hover:bg-accent flex h-7 items-center gap-2 rounded-md border px-3 text-xs">
          <span className="size-1.5 rounded-full bg-[var(--s-4xx)]" />
          Synthetic traffic · proxy listens on 127.0.0.1:9090 from M1
        </button>
      </div>

      <ToolButton icon={Gauge} label="Throttle" milestone="M5" />
      <ToolButton icon={OctagonPause} label="Breakpoints" milestone="M5" />
      <Separator orientation="vertical" className="mx-1 !h-5" />
      <ToolButton icon={Globe} label="System proxy" milestone="M4" />
      <ToolButton icon={ShieldCheck} label="Certificate" milestone="M2" />
      <ToolButton icon={Bot} label="MCP server" milestone="M7" />
      <ToolButton icon={Settings} label="Settings" milestone="M10" />
    </header>
  );
}
