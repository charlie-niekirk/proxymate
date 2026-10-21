import { useQuery } from "@tanstack/react-query";
import { Smartphone } from "lucide-react";

import type { DeviceAccess_Serialize } from "@/bindings";
import { CopyField, Path, ProgressCheck, QrCard, Section, Steps } from "@/components/DeviceSetup/parts";
import { backend } from "@/lib/backend";
import { activeDevices, MANIFEST_ATTRIBUTE, NETWORK_SECURITY_CONFIG, progress, splitProxy } from "@/lib/devices";
import { deviceAccessKey } from "@/lib/query";
import { useDevices } from "@/store/devices";

function useAccess() {
  return useQuery({ queryKey: deviceAccessKey, queryFn: () => backend.getDeviceAccess() }).data;
}

/** Server and port to type into the phone's Wi-Fi proxy settings. */
function ProxyFields({ access }: { access: DeviceAccess_Serialize | undefined }) {
  const parts = splitProxy(access?.proxy);
  if (!parts) return null;
  return (
    <div className="mt-1 grid grid-cols-[auto_1fr] items-center gap-x-2 gap-y-1">
      <span className="text-muted-foreground">Server</span>
      <CopyField value={parts.host} label="Copy server" />
      <span className="text-muted-foreground">Port</span>
      <CopyField value={parts.port} label="Copy port" />
    </div>
  );
}

/** "Waiting for a device…", then each allowed device's progress. */
function RemoteActivity({ device }: { device: string }) {
  const devices = activeDevices(useDevices((s) => s.snapshot.devices));
  return (
    <div className="flex flex-col gap-2 rounded-md border p-2.5" data-testid="remote-activity">
      {devices.length === 0 ? (
        <ProgressCheck progress="waiting" who={`your ${device}`} flows={0} />
      ) : (
        devices.map((d) => (
          <div key={d.ip} className="flex flex-col gap-1">
            <ProgressCheck progress={progress(d)} who={d.label} flows={d.flows} />
            {d.flows === 0 && d.onboarding && (
              <div className="text-muted-foreground ml-6">Opened the setup page; no proxied traffic yet.</div>
            )}
          </div>
        ))
      )}
    </div>
  );
}

export function IPhoneTab() {
  const access = useAccess();
  return (
    <div className="flex flex-col gap-4" data-testid="device-tab-iphone">
      <QrCard access={access} device="iPhone" />
      <Section title="Set up the iPhone">
        <Steps>
          <li>Join the same Wi-Fi network as this Mac.</li>
          <li>
            Open <Path>Settings › Wi-Fi</Path>, tap <Path>ⓘ</Path> next to the network, then{" "}
            <Path>Configure Proxy › Manual</Path>. Enter this server and port, and tap Save.
            <ProxyFields access={access} />
          </li>
          <li>
            Scan the QR code with the Camera (or open the setup URL in Safari). Proxymate asks on this Mac whether to
            allow the iPhone: click <Path>Allow</Path>.
          </li>
          <li>
            Tap <Path>Download profile</Path> and allow the download, then open{" "}
            <Path>Settings › Profile Downloaded</Path> and tap <Path>Install</Path>.
          </li>
          <li>
            Open <Path>Settings › General › About › Certificate Trust Settings</Path> and turn on full trust for the
            Proxymate CA. Without this step HTTPS still fails.
          </li>
        </Steps>
      </Section>
      <Section title="Status">
        <RemoteActivity device="iPhone" />
      </Section>
      <p className="text-muted-foreground">
        To stop, set the Wi-Fi proxy back to Off. Remove the profile in <Path>Settings › General › VPN &amp; Device
        Management</Path>.
      </p>
    </div>
  );
}

export function AndroidTab() {
  const access = useAccess();
  return (
    <div className="flex flex-col gap-4" data-testid="device-tab-android">
      <QrCard access={access} device="phone" />
      <Section title="Set up the Android device">
        <Steps>
          <li>Join the same Wi-Fi network as this Mac.</li>
          <li>
            Open <Path>Settings › Network &amp; internet › Internet</Path>, tap the gear next to the network, then the
            pencil (Edit). Under <Path>Advanced options › Proxy</Path> choose <Path>Manual</Path>, enter this hostname
            and port, and save.
            <ProxyFields access={access} />
          </li>
          <li>
            Scan the QR code (or open the setup URL in Chrome) and allow the device on this Mac. Tap{" "}
            <Path>Download certificate</Path>.
          </li>
          <li>
            Open <Path>Settings › Security &amp; privacy › More security settings › Encryption &amp; credentials ›
            Install a certificate › CA certificate</Path>, tap <Path>Install anyway</Path> and pick{" "}
            <code className="font-mono">proxymate-ca.crt</code> from Downloads.
          </li>
        </Steps>
      </Section>
      <Section title="Your app's debug builds">
        <p className="text-muted-foreground">
          Chrome trusts user CAs, but apps targeting Android 7 or later don&apos;t unless they opt in. Add this to your
          app as <code className="font-mono">res/xml/network_security_config.xml</code>; it applies to debug builds only.
        </p>
        <CopyField value={NETWORK_SECURITY_CONFIG} label="Copy network_security_config.xml" multiline />
        <p className="text-muted-foreground">Then point the manifest at it:</p>
        <CopyField value={MANIFEST_ATTRIBUTE} label="Copy manifest attribute" multiline />
      </Section>
      <Section title="Status">
        <RemoteActivity device="Android device" />
      </Section>
    </div>
  );
}

export function NoLanHint() {
  return (
    <div className="text-muted-foreground flex items-center gap-2">
      <Smartphone className="size-3.5" />
      Phones need device access; the Simulator and emulator don&apos;t.
    </div>
  );
}
