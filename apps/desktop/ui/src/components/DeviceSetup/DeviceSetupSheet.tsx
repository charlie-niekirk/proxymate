import { useQuery } from "@tanstack/react-query";
import { Check as CheckIcon, LoaderCircle, MonitorSmartphone, Trash2, X } from "lucide-react";
import { useState } from "react";

import type { DeviceClient_Deserialize } from "@/bindings";
import { EmulatorTab } from "@/components/DeviceSetup/EmulatorTab";
import { CopyField, DeviceMenu } from "@/components/DeviceSetup/parts";
import { AndroidTab, IPhoneTab, NoLanHint } from "@/components/DeviceSetup/PhoneTabs";
import { SimulatorTab } from "@/components/DeviceSetup/SimulatorTab";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { decideDevice, forgetDevice, setLanAccess, setLanInterface } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { deviceAccessKey } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useDevices } from "@/store/devices";
import { useUi, type DeviceTab } from "@/store/ui";

const TABS: { id: DeviceTab; label: string }[] = [
  { id: "simulator", label: "iOS Simulator" },
  { id: "iphone", label: "iPhone" },
  { id: "android", label: "Android" },
  { id: "emulator", label: "Emulator" },
];

/**
 * Device Setup Wizard, opened from "Listening on …": LAN access and the
 * interface devices use, then one tab per kind of device, then the LAN
 * clients Proxymate knows.
 */
export function DeviceSetupSheet() {
  const open = useUi((s) => s.deviceSetupOpen);
  const setOpen = useUi((s) => s.setDeviceSetupOpen);
  const tab = useUi((s) => s.deviceTab);
  const setTab = useUi((s) => s.setDeviceTab);
  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetContent
        side="right"
        className="w-[480px] gap-0 text-xs sm:max-w-[480px]"
        data-testid="device-setup"
        onOpenAutoFocus={(e) => e.preventDefault()}
      >
        <SheetHeader className="gap-1 border-b p-3 pr-10">
          <SheetTitle className="flex items-center gap-2 text-sm">
            <MonitorSmartphone className="size-4" />
            Device setup
          </SheetTitle>
          <SheetDescription className="text-xs">
            Send a phone, simulator or emulator through Proxymate and trust its certificate there.
          </SheetDescription>
        </SheetHeader>
        {open && <AccessBar />}
        <Tabs value={tab} onValueChange={(v) => setTab(v as DeviceTab)} className="min-h-0 flex-1 gap-0">
          <TabsList className="mx-3 mt-3 w-[calc(100%-1.5rem)]">
            {TABS.map((t) => (
              <TabsTrigger key={t.id} value={t.id} className="text-xs" data-testid={`device-tab-${t.id}-trigger`}>
                {t.label}
              </TabsTrigger>
            ))}
          </TabsList>
          <ScrollArea className="min-h-0 flex-1">
            <div className="flex flex-col gap-4 p-3">
              <TabsContent value="simulator">
                <SimulatorTab />
              </TabsContent>
              <TabsContent value="iphone">
                <IPhoneTab />
              </TabsContent>
              <TabsContent value="android">
                <AndroidTab />
              </TabsContent>
              <TabsContent value="emulator">
                <EmulatorTab />
              </TabsContent>
              <KnownDevices />
            </div>
          </ScrollArea>
        </Tabs>
      </SheetContent>
    </Sheet>
  );
}

/** "Allow devices on your network", the interface, and the proxy address. */
function AccessBar() {
  const { data: access } = useQuery({ queryKey: deviceAccessKey, queryFn: () => backend.getDeviceAccess() });
  const [busy, setBusy] = useState(false);
  const toggle = (on: boolean) => {
    setBusy(true);
    void setLanAccess(on).finally(() => setBusy(false));
  };
  const interfaces = access?.interfaces ?? [];
  return (
    <div className="bg-muted/30 flex flex-col gap-2 border-b p-3" data-testid="device-access">
      <label className="flex items-center gap-2">
        <Switch
          checked={access?.enabled ?? false}
          disabled={!access || busy}
          onCheckedChange={toggle}
          data-testid="lan-access"
        />
        <span className="font-medium">Allow devices on your network</span>
        {busy && <LoaderCircle className="text-muted-foreground size-3.5 animate-spin" />}
      </label>
      <div className="text-muted-foreground">
        {access?.enabled
          ? "Listening on every interface. A device's first connection asks you to allow it."
          : "Proxymate only listens on this Mac (127.0.0.1)."}
      </div>
      {!access?.enabled && <NoLanHint />}
      {access?.enabled && (
        <div className="grid grid-cols-[auto_1fr] items-center gap-x-2 gap-y-1.5">
          <span className="text-muted-foreground">Interface</span>
          {interfaces.length ? (
            <Select value={access.interface ?? undefined} onValueChange={(name) => void setLanInterface(name)}>
              <CopyMenu
                items={[
                  (() => {
                    const ip = interfaces.find((i) => i.name === access.interface)?.ip;
                    return ip && { label: "Copy IP address", text: ip, what: "IP address" };
                  })(),
                ]}
              >
                <SelectTrigger className="w-full" data-testid="lan-interface">
                  <SelectValue />
                </SelectTrigger>
              </CopyMenu>
              <SelectContent>
                {interfaces.map((i) => (
                  <SelectItem key={i.name} value={i.name} className="text-xs">
                    {i.name} · {i.ip}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <span className="text-[var(--s-4xx)]">No network interface with an IPv4 address</span>
          )}
          {access.proxy && (
            <>
              <span className="text-muted-foreground">Proxy</span>
              <CopyField value={access.proxy} label="Copy proxy address" />
            </>
          )}
        </div>
      )}
      {access?.error && <div className="text-[var(--s-failed)]">{access.error}</div>}
    </div>
  );
}

/** LAN clients: pending, allowed and denied, with their traffic so far. */
function KnownDevices() {
  const devices = useDevices((s) => s.snapshot.devices);
  if (!devices.length) return null;
  return (
    <section className="flex flex-col gap-2 border-t pt-3" data-testid="known-devices">
      <h3 className="text-foreground text-[13px] font-semibold">Devices on your network</h3>
      <ul className="flex flex-col divide-y rounded-md border">
        {devices.map((d) => (
          <DeviceRow key={d.ip} device={d} />
        ))}
      </ul>
    </section>
  );
}

function DeviceRow({ device }: { device: DeviceClient_Deserialize }) {
  const [error, setError] = useState<string | null>(null);
  const act = (action: Promise<unknown>) =>
    void action.catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  const decision = device.decision;
  const decide = (allow: boolean) => act(decideDevice(device.ip, allow, device.nickname ?? null));
  const forget = () => act(forgetDevice(device.ip));
  return (
    <DeviceMenu device={device} onDecide={decide} onForget={forget}>
      <li className="flex items-center gap-2 px-2 py-1.5">
        <span
          className={cn(
            "size-1.5 shrink-0 rounded-full",
            decision === "allowed" && "bg-[var(--s-2xx)]",
            decision === "pending" && "bg-[var(--s-4xx)]",
            decision === "denied" && "bg-[var(--s-5xx)]",
          )}
        />
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium">{device.label}</div>
          <div className="text-muted-foreground truncate">
            {decision === "pending"
              ? `Waiting for your answer${device.waiting ? ` · ${device.waiting} held` : ""}`
              : decision === "denied"
                ? "Denied"
                : `${device.flows} flows · ${device.https_decrypted} HTTPS decrypted`}
            {error && <span className="text-[var(--s-failed)]"> · {error}</span>}
          </div>
        </div>
        {decision !== "allowed" && (
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`Allow ${device.ip}`}
            onClick={() => decide(true)}
          >
            <CheckIcon />
          </Button>
        )}
        {decision !== "denied" && (
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`Deny ${device.ip}`}
            onClick={() => decide(false)}
          >
            <X />
          </Button>
        )}
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={`Forget ${device.ip}`}
          className="text-muted-foreground"
          onClick={forget}
        >
          <Trash2 />
        </Button>
      </li>
    </DeviceMenu>
  );
}
