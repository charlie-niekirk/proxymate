import { useQuery } from "@tanstack/react-query";
import { LoaderCircle, RefreshCw, ShieldPlus } from "lucide-react";
import { useState } from "react";

import type { SimInstallResult_Serialize } from "@/bindings";
import { Check, ProgressCheck, Section } from "@/components/DeviceSetup/parts";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { ContextMenuItem } from "@/components/ui/context-menu";
import { setSystemProxy } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { progress } from "@/lib/devices";
import { queryClient, simulatorKey, systemProxyKey } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useDevices } from "@/store/devices";

/**
 * The iOS Simulator uses this Mac's network settings, so it needs the
 * system proxy and the CA in each device's keychain (`simctl keychain
 * add-root-cert`), nothing else.
 */
export function SimulatorTab() {
  const { data: sim, isFetching } = useQuery({
    queryKey: simulatorKey,
    queryFn: () => backend.getSimulatorStatus(),
    refetchInterval: 4000,
    staleTime: 0,
  });
  const { data: sysproxy } = useQuery({ queryKey: systemProxyKey, queryFn: () => backend.getSystemProxyStatus() });
  const activity = useDevices((s) => s.snapshot.simulator);
  const [busy, setBusy] = useState<"install" | "proxy" | null>(null);
  const [results, setResults] = useState<SimInstallResult_Serialize[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const booted = sim?.booted ?? [];
  const allInstalled = booted.length > 0 && booted.every((d) => d.cert_installed);
  /** Installs on `udids`, or every booted simulator. */
  const install = (udids: string[] = []) => {
    setBusy("install");
    setError(null);
    backend
      .installSimulatorCert(udids)
      .then(setResults)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => {
        setBusy(null);
        void queryClient.invalidateQueries({ queryKey: simulatorKey });
      });
  };
  const proxyOn = () => {
    setBusy("proxy");
    void setSystemProxy(true).finally(() => setBusy(null));
  };

  return (
    <div className="flex flex-col gap-4" data-testid="device-tab-simulator">
      <Section>
        <div className="flex flex-col gap-2 rounded-md border p-2.5">
          <Check
            state={!sim ? "pending" : sim.available ? "ok" : "warn"}
            label={sim?.available === false ? "Xcode's simctl isn't available" : "Xcode simulator tools"}
          >
            {sim?.available === false && sim.detail}
          </Check>
          <Check
            state={!sim ? "pending" : booted.length ? "ok" : "warn"}
            label={
              booted.length
                ? `${booted.length} booted simulator${booted.length === 1 ? "" : "s"}`
                : "No simulator is booted"
            }
          >
            {sim?.available && !booted.length && "Start one from Xcode (Window › Devices and Simulators) or the Simulator app."}
            {sim?.available && booted.length > 0 && sim.detail}
          </Check>
          {booted.length > 0 && (
            <ul className="ml-6 flex flex-col gap-0.5" data-testid="simulator-devices">
              {booted.map((d) => {
                const result = results?.find((r) => r.udid === d.udid);
                return (
                  <CopyMenu
                    key={d.udid}
                    items={[
                      { label: "Copy name", text: d.name, what: "name" },
                      { label: "Copy UDID", text: d.udid, what: "UDID" },
                      { label: "Copy runtime", text: d.runtime, what: "runtime" },
                    ]}
                    extra={
                      <ContextMenuItem disabled={busy !== null} onSelect={() => install([d.udid])}>
                        Install certificate on this simulator
                      </ContextMenuItem>
                    }
                  >
                    <li className="flex items-center gap-2">
                      <span className="truncate">{d.name}</span>
                      <span className="text-muted-foreground">{d.runtime}</span>
                      <span
                        className={cn(
                          "ml-auto shrink-0",
                          d.cert_installed ? "text-[var(--s-2xx)]" : "text-muted-foreground",
                          result?.error && "text-[var(--s-failed)]",
                        )}
                      >
                        {result?.error ? "Failed" : d.cert_installed ? "Certificate installed" : "Not installed"}
                      </span>
                    </li>
                  </CopyMenu>
                );
              })}
            </ul>
          )}
          {results?.filter((r) => r.error).map((r) => (
            <div key={r.udid} className="ml-6 text-[var(--s-failed)]">
              {r.name}: {r.error}
            </div>
          ))}
          <Check
            state={!sysproxy ? "pending" : sysproxy.enabled ? "ok" : "warn"}
            label={sysproxy?.enabled ? "The system proxy points at Proxymate" : "The system proxy is off"}
          >
            {!sysproxy?.enabled && (
              <div className="flex flex-col items-start gap-1.5">
                <span>Simulators use this Mac&apos;s network settings, so they need the system proxy.</span>
                <Button variant="outline" size="xs" disabled={busy !== null || !sysproxy?.supported} onClick={proxyOn} data-testid="simulator-system-proxy">
                  {busy === "proxy" && <LoaderCircle className="animate-spin" />}
                  Turn on system proxy
                </Button>
              </div>
            )}
          </Check>
          <ProgressCheck progress={progress(activity)} who="the Simulator" flows={activity.flows} />
        </div>
      </Section>

      {error && <div className="text-[var(--s-failed)]">{error}</div>}

      <div className="flex items-center gap-2">
        <Button
          size="sm"
          disabled={busy !== null || !booted.length}
          onClick={() => install()}
          data-testid="simulator-install"
        >
          {busy === "install" ? <LoaderCircle className="animate-spin" /> : <ShieldPlus />}
          {allInstalled ? "Install again" : "Install to all booted"}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void queryClient.invalidateQueries({ queryKey: simulatorKey })}
          disabled={isFetching}
        >
          <RefreshCw className={cn(isFetching && "animate-spin")} />
          Refresh
        </Button>
      </div>

      <Section title="How it works">
        <p className="text-muted-foreground">
          Proxymate runs <code className="font-mono">xcrun simctl keychain &lt;device&gt; add-root-cert</code> for each
          booted simulator, which trusts the CA for every app on it. Quit and reopen apps that were already running,
          then open an https:// page in Safari. A simulator you erase or create later needs installing again.
        </p>
      </Section>
    </div>
  );
}
