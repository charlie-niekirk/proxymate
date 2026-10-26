import { useQuery } from "@tanstack/react-query";
import { Copy, Globe, LoaderCircle } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { copyTerminalEnv, setSystemProxy } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { proxyStatusKey, systemProxyKey } from "@/lib/query";
import { cn } from "@/lib/utils";

/**
 * Toolbar popover for the macOS system proxy (web + secure web on every
 * active network service), plus "Copy terminal env" for tools that ignore
 * it. The previous settings are always put back on quit or crash.
 */
export function SystemProxyMenu() {
  const { data: status } = useQuery({ queryKey: systemProxyKey, queryFn: () => backend.getSystemProxyStatus() });
  const { data: proxy } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const enabled = status?.enabled ?? false;

  const toggle = () => {
    setBusy(true);
    void setSystemProxy(!enabled).finally(() => setBusy(false));
  };

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label="System proxy"
          aria-pressed={enabled}
          data-testid="system-proxy-menu"
          className={cn(enabled ? "text-[var(--s-2xx)]" : "text-muted-foreground")}
        >
          <Globe />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-0 text-xs">
        <div className="border-b px-3 py-2">
          <div className="flex items-center gap-2 text-sm font-semibold">
            <span
              className="size-1.5 rounded-full"
              style={{ backgroundColor: enabled ? "var(--s-2xx)" : "var(--muted-foreground)" }}
            />
            System proxy {enabled ? "on" : "off"}
          </div>
          <div className="text-muted-foreground">
            Routes Safari, Chrome and other apps that follow the macOS proxy settings through{" "}
            {proxy?.listening ?? "Proxymate"}. Your previous settings come back on quit, even after a crash.
          </div>
        </div>
        <div className="flex flex-col gap-2 px-3 py-2">
          {status?.services.length ? (
            <div>
              <span className="text-muted-foreground">Services: </span>
              {status.services.join(", ")}
            </div>
          ) : null}
          {status && !status.supported && (
            <div className="text-muted-foreground">Not available on this platform yet.</div>
          )}
          {status?.error && <div className="text-[var(--s-failed)]">{status.error}</div>}
          <Button
            size="sm"
            variant={enabled ? "outline" : "default"}
            disabled={busy || !status?.supported || !proxy?.listening}
            onClick={toggle}
            data-testid="system-proxy-toggle"
          >
            {busy && <LoaderCircle className="animate-spin" />}
            {enabled ? "Turn off" : "Use as system proxy"}
          </Button>
        </div>
        <div className="flex items-center justify-between gap-2 border-t px-3 py-2">
          <span className="text-muted-foreground">For curl, Node, Python and other terminal tools</span>
          <Button
            variant="outline"
            size="xs"
            onClick={() =>
              void copyTerminalEnv(true).then((ok) => {
                if (!ok) return;
                setCopied(true);
                setTimeout(() => setCopied(false), 1500);
              })
            }
            data-testid="copy-terminal-env"
          >
            <Copy />
            {copied ? "Copied" : "Copy terminal env"}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
