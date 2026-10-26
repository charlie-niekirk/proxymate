import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { ProxySettings_Serialize } from "@/bindings";
import { CopyField } from "@/components/DeviceSetup/parts";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { setLanAccess, setLanInterface, setSystemProxy, errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { deviceAccessKey, proxySettingsKey, proxyStatusKey, queryClient, systemProxyKey } from "@/lib/query";

import { Note, NumberField, Page, Section } from "./parts";

/** Settings › Proxy: the port, LAN access, the system proxy and a proxied terminal. */
export function ProxyPage() {
  const { data: settings } = useQuery({ queryKey: proxySettingsKey, queryFn: () => backend.getProxySettings() });
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const { data: access } = useQuery({ queryKey: deviceAccessKey, queryFn: () => backend.getDeviceAccess() });
  const { data: sysproxy } = useQuery({ queryKey: systemProxyKey, queryFn: () => backend.getSystemProxyStatus() });
  const { data: env } = useQuery({ queryKey: ["terminal-env"], queryFn: () => backend.getTerminalEnv() });
  const lanIp = access?.interfaces.find((i) => i.name === access.interface)?.ip ?? null;
  const [error, setError] = useState<string | null>(null);
  const [lanError, setLanError] = useState<string | null>(null);

  return (
    <Page testId="settings-proxy">
      <Section
        title="Listening"
        description="Proxymate tries this port first, then the nine after it. Changing it restarts the listener; open connections finish on the old one."
      >
        <CopyMenu
          items={[
            status?.listening && { label: "Copy address", text: status.listening, what: "proxy address" },
            status?.error && { label: "Copy error", text: status.error, what: "error" },
          ]}
        >
          <div className="flex items-center gap-2 rounded-md border px-2 py-1.5" data-testid="proxy-listening">
            <span
              className="size-1.5 shrink-0 rounded-full"
              style={{ backgroundColor: status?.listening ? "var(--s-2xx)" : "var(--s-5xx)" }}
            />
            <span className="min-w-0 flex-1 truncate font-mono text-[11px]">
              {status?.listening ? `Listening on ${status.listening}` : `Not running: ${status?.error ?? "stopped"}`}
            </span>
          </div>
        </CopyMenu>
        {settings && <PortForm key={settings.port} settings={settings} onError={setError} />}
        <Note error={error} />
      </Section>

      <Section
        title="LAN access"
        description="Listen on every network interface so phones and other computers can use the proxy. Each new device has to be allowed first (Settings › Clients)."
        aside={
          <Switch
            checked={access?.enabled ?? false}
            onCheckedChange={(on) => {
              setLanError(null);
              setLanAccess(on).catch((e: unknown) => setLanError(errorText(e)));
            }}
            aria-label="LAN access"
            data-testid="settings-lan"
          />
        }
      >
        {access?.enabled && access.interfaces.length > 0 && (
          <label className="grid grid-cols-[180px_1fr] items-center gap-3">
            <span>Address shown to devices</span>
            <Select value={access.interface ?? undefined} onValueChange={(name) => void setLanInterface(name)}>
              <CopyMenu
                items={[
                  lanIp && { label: "Copy IP address", text: lanIp, what: "IP address" },
                  access.proxy && { label: "Copy address and port", text: access.proxy, what: "proxy address" },
                ]}
              >
                <SelectTrigger size="sm" className="w-64">
                  <SelectValue placeholder="Pick an interface" />
                </SelectTrigger>
              </CopyMenu>
              <SelectContent>
                {access.interfaces.map((i) => (
                  <SelectItem key={i.name} value={i.name}>
                    {i.ip} <span className="text-muted-foreground">({i.name})</span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
        )}
        <Note error={lanError ?? access?.error ?? null} />
      </Section>

      <Section
        title="System proxy"
        description="Point this Mac's HTTP and HTTPS proxy at Proxymate, for every app that follows the system setting. It's put back when Proxymate quits, even after a crash."
        aside={
          <Switch
            checked={sysproxy?.enabled ?? false}
            disabled={sysproxy ? !sysproxy.supported : true}
            onCheckedChange={(on) => void setSystemProxy(on)}
            aria-label="System proxy"
            data-testid="settings-sysproxy"
          />
        }
      >
        <Note error={sysproxy?.error ?? null} />
      </Section>

      <Section
        title="Proxied terminal"
        description="Paste into a shell to send curl, Node, Python and friends through Proxymate. The CA bundle holds this Mac's trusted roots plus Proxymate's CA."
      >
        {env && <CopyField value={env} multiline testId="settings-terminal-env" />}
      </Section>
    </Page>
  );
}

function PortForm({ settings, onError }: { settings: ProxySettings_Serialize; onError: (e: string | null) => void }) {
  const [port, setPort] = useState(settings.port);
  const [busy, setBusy] = useState(false);
  const apply = async () => {
    onError(null);
    setBusy(true);
    try {
      queryClient.setQueryData(proxySettingsKey, await backend.setProxySettings({ ...settings, port }));
    } catch (e) {
      onError(errorText(e));
    } finally {
      setBusy(false);
      await queryClient.invalidateQueries({ queryKey: proxyStatusKey });
      await queryClient.invalidateQueries({ queryKey: deviceAccessKey });
      await queryClient.invalidateQueries({ queryKey: ["terminal-env"] });
    }
  };
  return (
    <form
      className="flex items-end gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        void apply();
      }}
    >
      <NumberField label="Port" value={port} onChange={setPort} min={1024} max={65535} testId="settings-port" />
      <Button type="submit" variant="outline" size="sm" disabled={busy || port === settings.port || !Number.isFinite(port)}>
        {busy ? "Restarting…" : "Apply"}
      </Button>
    </form>
  );
}
