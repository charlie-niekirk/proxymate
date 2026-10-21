import { useQuery } from "@tanstack/react-query";
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

import type { ProxyStatus_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { clearFlows, setRecording } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { proxyStatusKey } from "@/lib/query";

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
 * window. Controls for later milestones are placeholders.
 */
export function Toolbar() {
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const recording = status?.recording ?? true;
  return (
    <header
      data-tauri-drag-region
      className="bg-pane-toolbar flex h-[52px] shrink-0 items-center gap-1 border-b px-2"
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={recording ? "Pause recording" : "Resume recording"}
            aria-pressed={recording}
            onClick={() => void setRecording(!recording)}
          >
            <Circle className={recording ? "fill-[var(--m-delete)] text-[var(--m-delete)]" : "text-muted-foreground"} />
          </Button>
        </TooltipTrigger>
        <TooltipContent>{recording ? "Recording · click to pause" : "Paused · traffic is forwarded but not recorded"}</TooltipContent>
      </Tooltip>
      <ToolButton icon={Plus} label="Compose" milestone="M8" />
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="Clear" onClick={() => void clearFlows()}>
            <Trash2 />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Clear all flows</TooltipContent>
      </Tooltip>

      <div data-tauri-drag-region className="flex flex-1 justify-center">
        <ListeningStatus status={status} />
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

function ListeningStatus({ status }: { status: ProxyStatus_Serialize | undefined }) {
  const [dot, text] = !status
    ? ["var(--s-1xx)", "Starting proxy…"]
    : status.listening
      ? [status.recording ? "var(--s-2xx)" : "var(--s-paused)", `Listening on ${status.listening}${status.recording ? "" : " · paused"}`]
      : ["var(--s-5xx)", `Proxy not running: ${status.error ?? "stopped"}`];
  const url = status?.listening ? `http://${status.listening}` : null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          className="text-muted-foreground hover:bg-accent flex h-7 max-w-[520px] items-center gap-2 rounded-md border px-3 text-xs"
          data-testid="listening-status"
          onClick={() => url && void navigator.clipboard?.writeText(url)}
        >
          <span className="size-1.5 shrink-0 rounded-full" style={{ backgroundColor: dot }} />
          <span className="truncate">{text}</span>
        </button>
      </TooltipTrigger>
      <TooltipContent>{url ? `Copy ${url} · device setup arrives in M9` : text}</TooltipContent>
    </Tooltip>
  );
}
