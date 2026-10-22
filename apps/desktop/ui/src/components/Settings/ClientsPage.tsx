import { useQuery } from "@tanstack/react-query";
import { Smartphone } from "lucide-react";
import { useState } from "react";

import type { DeviceClient_Deserialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { deviceAccessKey } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useDevices } from "@/store/devices";
import { useUi } from "@/store/ui";

import { Note, Page, Section } from "./parts";

const LABEL: Record<DeviceClient_Deserialize["decision"], string> = {
  pending: "Waiting for an answer",
  allowed: "Allowed",
  denied: "Denied",
};

/**
 * Settings › Clients: LAN devices that asked to use the proxy. Allowed
 * ones are recorded under their nickname; denied ones are refused;
 * forgotten ones are asked about again.
 */
export function ClientsPage() {
  const devices = useDevices((s) => s.snapshot.devices);
  const { data: access } = useQuery({ queryKey: deviceAccessKey, queryFn: () => backend.getDeviceAccess() });
  const openSettings = useUi((s) => s.openSettings);
  const [error, setError] = useState<string | null>(null);
  const run = (p: Promise<unknown>) => {
    setError(null);
    p.catch((e: unknown) => setError(errorText(e)));
  };
  return (
    <Page testId="settings-clients">
      <Section
        title="LAN clients"
        description={
          access?.enabled ? (
            "Devices on your network that connected to Proxymate. Loopback and this Mac's own addresses are always allowed."
          ) : (
            <>
              LAN access is off, so only this Mac can use the proxy.{" "}
              <button className="text-foreground underline underline-offset-2" onClick={() => openSettings("proxy")}>
                Turn it on in Proxy
              </button>
              .
            </>
          )
        }
      >
        <Note error={error} />
        {devices.length === 0 ? (
          <p className="text-muted-foreground rounded-md border px-2 py-3 text-center italic">No devices have connected yet</p>
        ) : (
          <ul className="divide-y rounded-md border" data-testid="clients-list">
            {devices.map((d) => (
              <ClientRow
                key={d.ip}
                device={d}
                onDecide={(allow, nickname) => run(backend.decideDevice(d.ip, allow, nickname))}
                onForget={() => run(backend.forgetDevice(d.ip))}
              />
            ))}
          </ul>
        )}
      </Section>
    </Page>
  );
}

function ClientRow({
  device,
  onDecide,
  onForget,
}: {
  device: DeviceClient_Deserialize;
  onDecide: (allow: boolean, nickname: string | null) => void;
  onForget: () => void;
}) {
  const [nickname, setNickname] = useState(device.nickname ?? "");
  const changed = (nickname.trim() || null) !== (device.nickname ?? null);
  return (
    <li className="flex items-center gap-2 px-2 py-1.5" data-testid="client-row">
      <Smartphone className="text-muted-foreground size-3.5 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="font-mono text-[11px]">{device.ip}</span>
          <span
            className={cn(
              "text-2xs",
              device.decision === "allowed" && "text-[var(--s-2xx)]",
              device.decision === "denied" && "text-[var(--s-failed)]",
              device.decision === "pending" && "text-[var(--s-4xx)]",
            )}
          >
            {LABEL[device.decision]}
          </span>
        </div>
        <div className="text-muted-foreground text-2xs">
          {device.flows} flows · {device.https_decrypted} decrypted
          {device.https_refused > 0 && ` · ${device.https_refused} refused (CA not trusted yet?)`}
        </div>
      </div>
      <Input
        value={nickname}
        onChange={(e) => setNickname(e.target.value)}
        placeholder="Nickname"
        aria-label={`Nickname for ${device.ip}`}
        className="w-40"
      />
      {device.decision !== "allowed" || changed ? (
        <Button size="xs" onClick={() => onDecide(true, nickname.trim() || null)}>
          {device.decision === "allowed" ? "Save" : "Allow"}
        </Button>
      ) : (
        <Button size="xs" variant="outline" onClick={() => onDecide(false, null)}>
          Deny
        </Button>
      )}
      {device.decision === "pending" && (
        <Button size="xs" variant="outline" onClick={() => onDecide(false, null)}>
          Deny
        </Button>
      )}
      <Button size="xs" variant="ghost" onClick={onForget}>
        Forget
      </Button>
    </li>
  );
}
