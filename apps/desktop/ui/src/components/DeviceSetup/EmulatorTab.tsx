import { useQuery } from "@tanstack/react-query";
import { Download, LoaderCircle, RefreshCw } from "lucide-react";
import { useState } from "react";

import { Check, CopyField, Path, ProgressCheck, Section, Steps } from "@/components/DeviceSetup/parts";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { setAndroidProxy } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { emulatorCommands, progress } from "@/lib/devices";
import { androidKey, deviceAccessKey, queryClient } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useDevices } from "@/store/devices";

/**
 * The Android emulator reaches this Mac's loopback at 10.0.2.2, so it
 * needs no LAN access: point its global proxy there over adb (cleared again
 * on quit) and install the CA as a user certificate.
 */
export function EmulatorTab() {
  const { data: status, isFetching } = useQuery({
    queryKey: androidKey,
    queryFn: () => backend.getAndroidStatus(),
    refetchInterval: 4000,
    staleTime: 0,
  });
  const { data: access } = useQuery({ queryKey: deviceAccessKey, queryFn: () => backend.getDeviceAccess() });
  const activity = useDevices((s) => s.snapshot.emulator);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const emulators = (status?.devices ?? []).filter((d) => d.emulator);
  const commands = emulatorCommands(access?.port);

  const run = (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    setNote(null);
    action()
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setBusy(null));
  };

  return (
    <div className="flex flex-col gap-4" data-testid="device-tab-emulator">
      <div className="flex flex-col gap-2 rounded-md border p-2.5">
        <Check state={!status ? "pending" : status.available ? "ok" : "warn"} label={status?.available === false ? "adb wasn't found" : "Android SDK platform-tools"}>
          {status?.detail}
        </Check>
        <Check
          state={!status ? "pending" : emulators.length ? "ok" : "warn"}
          label={emulators.length ? `${emulators.length} running emulator${emulators.length === 1 ? "" : "s"}` : "No emulator is running"}
        >
          {status?.available && !emulators.length && "Start one from Android Studio's Device Manager."}
        </Check>
        {emulators.map((d) => {
          const ours = d.proxy === status?.emulator_proxy;
          return (
            <div key={d.serial} className="ml-6 flex items-center gap-2" data-testid="emulator-row">
              <div className="min-w-0 flex-1">
                <div className="truncate">{d.model ?? d.serial}</div>
                <div className="text-muted-foreground truncate">
                  {d.serial} · {d.state !== "device" ? d.state : d.proxy ? `proxy ${d.proxy}` : "no proxy"}
                </div>
              </div>
              <Button
                variant="outline"
                size="xs"
                disabled={busy !== null || d.state !== "device"}
                onClick={() =>
                  run(`push:${d.serial}`, async () => {
                    const path = await backend.pushAndroidCert(d.serial);
                    setNote(`Copied to ${path} on ${d.serial}.`);
                  })
                }
              >
                {busy === `push:${d.serial}` ? <LoaderCircle className="animate-spin" /> : <Download />}
                Push certificate
              </Button>
              <label className="flex items-center gap-1.5">
                <Switch
                  size="sm"
                  checked={ours}
                  disabled={busy !== null || d.state !== "device"}
                  onCheckedChange={(on) => run(`proxy:${d.serial}`, () => setAndroidProxy(d.serial, on))}
                  aria-label={`Use Proxymate as ${d.serial}'s proxy`}
                />
                Proxy
              </label>
            </div>
          );
        })}
        <ProgressCheck progress={progress(activity)} who="the emulator" flows={activity.flows} />
      </div>
      {note && <div className="text-[var(--s-2xx)]">{note}</div>}
      {error && <div className="text-[var(--s-failed)]">{error}</div>}
      <div>
        <Button
          variant="ghost"
          size="sm"
          disabled={isFetching}
          onClick={() => void queryClient.invalidateQueries({ queryKey: androidKey })}
        >
          <RefreshCw className={cn(isFetching && "animate-spin")} />
          Refresh
        </Button>
      </div>

      <Section title="Set up the emulator">
        <Steps>
          <li>
            Turn on <Path>Proxy</Path> for the emulator above. It is set to{" "}
            <code className="font-mono">{status?.emulator_proxy ?? "10.0.2.2:9090"}</code>, the emulator&apos;s address
            for this Mac, and cleared again when Proxymate quits.
          </li>
          <li>
            Click <Path>Push certificate</Path>, then on the emulator open{" "}
            <Path>Settings › Security &amp; privacy › More security settings › Encryption &amp; credentials › Install a
            certificate › CA certificate</Path> and pick <code className="font-mono">proxymate-ca.crt</code> from
            Downloads.
          </li>
          <li>
            For your own app, add the debug-only <code className="font-mono">network_security_config.xml</code> from the
            Android tab.
          </li>
        </Steps>
      </Section>
      <Section title="From a terminal">
        <CopyField value={commands.set} label="Copy set-proxy command" />
        <CopyField value={commands.clear} label="Copy clear-proxy command" />
      </Section>
    </div>
  );
}
