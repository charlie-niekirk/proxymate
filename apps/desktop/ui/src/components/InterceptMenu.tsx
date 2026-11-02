import { useQuery } from "@tanstack/react-query";
import { AppWindow, ArchiveX, Cookie, Crosshair, SquareTerminal } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { intercept, setBuiltinRule } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { INTERCEPT_TARGETS } from "@/lib/capture";
import { interceptAppsKey, interceptsKey, rulesKey } from "@/lib/query";
import { NO_CACHE_RULE_ID, NO_COOKIES_RULE_ID } from "@/lib/rules";
import { cn } from "@/lib/utils";

/**
 * "Intercept": terminals and browsers started already pointed at the
 * proxy and trusting its CA (M21). Their flows show as `<App> (Intercepted)`.
 */
export function InterceptMenu() {
  const [open, setOpen] = useState(false);
  const { data: apps } = useQuery({ queryKey: interceptAppsKey, queryFn: () => backend.listInterceptApps() });
  const { data: running } = useQuery({
    queryKey: interceptsKey,
    queryFn: () => backend.listIntercepts(),
    // Launches end when their app quits.
    refetchInterval: open ? 2000 : 10_000,
  });
  const count = running?.length ?? 0;
  const installed = (target: string) => apps?.find((a) => a.target === target)?.installed ?? true;
  const start = (target: (typeof INTERCEPT_TARGETS)[number]["target"]) => {
    setOpen(false);
    void intercept(target);
  };
  const terminals = INTERCEPT_TARGETS.filter((t) => t.terminal);
  const browsers = INTERCEPT_TARGETS.filter((t) => !t.terminal);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button variant="ghost" size={count ? "sm" : "icon"} aria-label="Intercept" data-testid="intercept-menu">
              <Crosshair />
              {count > 0 && <span className="text-xs tabular-nums">{count}</span>}
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Intercept: a terminal or browser that goes through Proxymate</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-80 p-1 text-xs" data-testid="intercept-popover">
        <div className="text-muted-foreground px-2 py-1">New window with the proxy and CA set</div>
        {terminals.map((t) => (
          <Item
            key={t.target}
            icon={<SquareTerminal className="size-3.5" />}
            label={t.name}
            detail="curl, node, python, git…"
            disabled={!installed(t.target)}
            onClick={() => start(t.target)}
            testId={`intercept-${t.target}`}
          />
        ))}
        <div className="text-muted-foreground mt-1 border-t px-2 pt-2 pb-1">Browser with a fresh profile</div>
        {browsers.map((t) => (
          <Item
            key={t.target}
            icon={<AppWindow className="size-3.5" />}
            label={t.name}
            detail={installed(t.target) ? "" : "not installed"}
            disabled={!installed(t.target)}
            onClick={() => start(t.target)}
            testId={`intercept-${t.target}`}
          />
        ))}
        {count > 0 && (
          <>
            <div className="text-muted-foreground mt-1 border-t px-2 pt-2 pb-1">Running</div>
            {running?.map((r) => (
              <div key={r.id} className="flex items-center gap-2 px-2 py-1" data-testid="intercept-running">
                <span className="size-1.5 shrink-0 rounded-full bg-[var(--s-2xx)]" />
                <span className="flex-1 truncate">{r.name} (Intercepted)</span>
                <span className="text-muted-foreground font-mono text-2xs">{r.pid ? `pid ${r.pid}` : "starting…"}</span>
              </div>
            ))}
          </>
        )}
      </PopoverContent>
    </Popover>
  );
}

function Item({
  icon,
  label,
  detail,
  disabled,
  onClick,
  testId,
}: {
  icon: React.ReactNode;
  label: string;
  detail: string;
  disabled: boolean;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      data-testid={testId}
      className="hover:bg-accent flex w-full items-center gap-2 rounded-sm px-2 py-1 text-left disabled:pointer-events-none disabled:opacity-50"
    >
      {icon}
      <span className="flex-1">{label}</span>
      <span className="text-muted-foreground text-2xs">{detail}</span>
    </button>
  );
}

/**
 * No Caching and Block Cookies: the built-in rules behind two toolbar
 * toggles, like the throttle (M21).
 */
export function NoiseToggles() {
  const { data: rules } = useQuery({ queryKey: rulesKey, queryFn: () => backend.listRules() });
  const noCache = rules?.find((r) => r.id === NO_CACHE_RULE_ID)?.enabled ?? false;
  const noCookies = rules?.find((r) => r.id === NO_COOKIES_RULE_ID)?.enabled ?? false;
  return (
    <>
      <Toggle
        on={noCache}
        label="No Caching"
        tip={noCache ? "No Caching is on: validators stripped, responses not stored · click to turn off" : "No Caching: strip If-None-Match, ETag and friends so every response is fresh"}
        icon={<ArchiveX />}
        onClick={() => void setBuiltinRule(NO_CACHE_RULE_ID, !noCache)}
        testId="no-cache-toggle"
      />
      <Toggle
        on={noCookies}
        label="Block Cookies"
        tip={noCookies ? "Block Cookies is on: Cookie and Set-Cookie removed · click to turn off" : "Block Cookies: remove Cookie and Set-Cookie headers"}
        icon={<Cookie />}
        onClick={() => void setBuiltinRule(NO_COOKIES_RULE_ID, !noCookies)}
        testId="no-cookies-toggle"
      />
    </>
  );
}

function Toggle({
  on,
  label,
  tip,
  icon,
  onClick,
  testId,
}: {
  on: boolean;
  label: string;
  tip: string;
  icon: React.ReactNode;
  onClick: () => void;
  testId: string;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size={on ? "sm" : "icon"}
          aria-label={label}
          aria-pressed={on}
          onClick={onClick}
          data-testid={testId}
          className={cn(on ? "text-[var(--s-4xx)]" : "text-muted-foreground")}
        >
          {icon}
          {on && <span className="text-xs">{label}</span>}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  );
}
