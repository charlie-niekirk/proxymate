import { useQuery } from "@tanstack/react-query";
import { Circle, FileCode2, ListChecks, Plus, Settings, ShieldAlert, Smartphone, Trash2 } from "lucide-react";

import type { ProxyStatus_Serialize } from "@/bindings";
import { BreakpointBadge } from "@/components/Breakpoints/BreakpointBadge";
import { CertificateMenu } from "@/components/CertificateMenu";
import { ThrottleMenu } from "@/components/Rules/ThrottleMenu";
import { McpIndicator } from "@/components/Settings/McpIndicator";
import { SystemProxyMenu } from "@/components/SystemProxyMenu";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { ContextMenuItem } from "@/components/ui/context-menu";
import { useTrustStatus } from "@/hooks/useTrustStatus";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { clearFlows, setRecording } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { shortcutLabel } from "@/lib/commands";
import { useDevices } from "@/store/devices";
import { proxyStatusKey } from "@/lib/query";
import { LIVE_STORE, useUi } from "@/store/ui";

/**
 * The toolbar, drawn in the overlay title bar area. Empty space drags the
 * window.
 */
export function Toolbar() {
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const recording = status?.recording ?? true;
  const openCompose = useUi((s) => s.openCompose);
  const live = useUi((s) => s.store === LIVE_STORE);
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
        <TooltipContent>
          {recording ? "Recording · click to pause" : "Paused · traffic is forwarded but not recorded"} · {shortcutLabel("e")}
        </TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="Compose" onClick={() => openCompose()} data-testid="compose-button">
            <Plus />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Compose a request · ⌘N</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="Clear" disabled={!live} onClick={() => void clearFlows()}>
            <Trash2 />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Clear all flows · {shortcutLabel("Backspace")}</TooltipContent>
      </Tooltip>

      <div data-tauri-drag-region className="flex flex-1 items-center justify-center gap-2">
        <ListeningStatus status={status} />
        <WaitingDevices />
      </div>

      <RulesButton />
      <ScriptsButton />
      <ThrottleMenu />
      <BreakpointBadge />
      <Separator orientation="vertical" className="mx-1 !h-5" />
      <TrustWarning listening={!!status?.listening} />
      <SystemProxyMenu />
      <CertificateMenu />
      <McpIndicator />
      <SettingsButton />
    </header>
  );
}

function SettingsButton() {
  const open = useUi((s) => s.openSettings);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="Settings" onClick={() => open()} data-testid="settings-button">
          <Settings />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Settings · {shortcutLabel(",")}</TooltipContent>
    </Tooltip>
  );
}

function RulesButton() {
  const open = useUi((s) => s.openRulesEditor);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="Rules" onClick={() => open()} data-testid="rules-button">
          <ListChecks />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Rules: Map Local, rewrites, breakpoints, Map Remote, block</TooltipContent>
    </Tooltip>
  );
}

function ScriptsButton() {
  const open = useUi((s) => s.openScripts);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="Scripts" onClick={() => open()} data-testid="scripts-button">
          <FileCode2 />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Scripts: decode bodies, rewrite and mock with JavaScript</TooltipContent>
    </Tooltip>
  );
}

/** Shown while the OS doesn't trust the CA (checked at launch). */
function TrustWarning({ listening }: { listening: boolean }) {
  const { data: trust } = useTrustStatus();
  const open = useUi((s) => s.setCertWizardOpen);
  if (!listening || !trust || trust.verified || trust.keychain === "unsupported") return null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className="border-[var(--s-4xx)]/50 text-[var(--s-4xx)] hover:text-[var(--s-4xx)]"
          onClick={() => open(true)}
          data-testid="trust-warning"
        >
          <ShieldAlert />
          Certificate not trusted
        </Button>
      </TooltipTrigger>
      <TooltipContent>HTTPS traffic can&apos;t be decrypted yet · open the certificate wizard</TooltipContent>
    </Tooltip>
  );
}

/** LAN devices waiting for an answer whose prompt was closed: click to answer. */
function WaitingDevices() {
  const devices = useDevices((s) => s.snapshot.devices);
  const reopen = useDevices((s) => s.reopen);
  const dismissed = useDevices((s) => s.dismissed);
  const waiting = devices.filter((d) => d.decision === "pending" && dismissed.has(d.ip)).length;
  if (waiting === 0) return null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className="border-[var(--s-4xx)]/50 text-[var(--s-4xx)] hover:text-[var(--s-4xx)]"
          onClick={reopen}
          data-testid="devices-waiting"
        >
          <Smartphone />
          {waiting} device{waiting === 1 ? "" : "s"} waiting
        </Button>
      </TooltipTrigger>
      <TooltipContent>A device on your network wants to use the proxy · click to allow or deny it</TooltipContent>
    </Tooltip>
  );
}

const copyTerminalEnvText = () => backend.getTerminalEnv();

function ListeningStatus({ status }: { status: ProxyStatus_Serialize | undefined }) {
  const openDeviceSetup = useUi((s) => s.setDeviceSetupOpen);
  const openSettings = useUi((s) => s.openSettings);
  const failed = !!status && !status.listening;
  const [dot, text] = !status
    ? ["var(--s-1xx)", "Starting proxy…"]
    : status.listening
      ? [status.recording ? "var(--s-2xx)" : "var(--s-paused)", `Listening on ${status.listening}${status.recording ? "" : " · paused"}`]
      : ["var(--s-5xx)", `Proxy not running: ${status.error ?? "stopped"}`];
  const addr = status?.listening ?? null;
  return (
    <Tooltip>
      <CopyMenu
        testId="listening-menu"
        items={[
          addr && { label: "Copy address", text: addr, what: "proxy address", testId: "copy-proxy-address" },
          addr && { label: "Copy curl example", text: `curl -x http://${addr} http://example.com/`, what: "command" },
          addr && { label: "Copy terminal environment", text: copyTerminalEnvText, what: "terminal environment" },
          status?.error && { label: "Copy error", text: status.error, what: "error" },
        ]}
        extra={
          <>
            <ContextMenuItem onSelect={() => openDeviceSetup(true)}>Device setup…</ContextMenuItem>
            <ContextMenuItem onSelect={() => openSettings("proxy")}>Proxy settings…</ContextMenuItem>
          </>
        }
      >
        <TooltipTrigger asChild>
          <button
            className="text-muted-foreground hover:bg-accent flex h-7 max-w-[520px] items-center gap-2 rounded-md border px-3 text-xs"
            data-testid="listening-status"
            onClick={() => (failed ? openSettings("proxy") : openDeviceSetup(true))}
          >
            <span className="size-1.5 shrink-0 rounded-full" style={{ backgroundColor: dot }} />
            <span className="truncate">{text}</span>
          </button>
        </TooltipTrigger>
      </CopyMenu>
      <TooltipContent className="max-w-96">
        {failed ? `${status?.error ?? "The proxy isn't running."} Click to change the port.` : "Device setup · connect a phone, simulator or emulator"}
      </TooltipContent>
    </Tooltip>
  );
}
