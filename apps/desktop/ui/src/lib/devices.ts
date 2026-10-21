import type { DeviceClient_Deserialize, LocalActivity } from "@/bindings";

/** How far a device has got, for the wizard's live checks. */
export type Progress = "waiting" | "connected" | "untrusted" | "decrypting";

/** From a device's flow counts: has it sent anything, and is its HTTPS decrypted? */
export function progress(activity: Pick<LocalActivity, "flows" | "https_decrypted" | "https_refused">): Progress {
  if (activity.https_decrypted > 0) return "decrypting";
  if (activity.https_refused > 0) return "untrusted";
  return activity.flows > 0 ? "connected" : "waiting";
}

/** Allowed LAN devices that have sent traffic or fetched the setup page, most recent first. */
export function activeDevices(devices: readonly DeviceClient_Deserialize[]): DeviceClient_Deserialize[] {
  return devices
    .filter((d) => d.decision === "allowed" && (d.flows > 0 || d.onboarding))
    .sort((a, b) => (b.last_seen_ms ?? 0) - (a.last_seen_ms ?? 0));
}

/** `host:port` split for the Wi-Fi proxy fields. */
export function splitProxy(proxy: string | null | undefined): { host: string; port: string } | null {
  if (!proxy) return null;
  const at = proxy.lastIndexOf(":");
  if (at <= 0) return null;
  return { host: proxy.slice(0, at), port: proxy.slice(at + 1) };
}

/** `res/xml/network_security_config.xml` that trusts user CAs in debug builds only. */
export const NETWORK_SECURITY_CONFIG = `<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <debug-overrides>
        <trust-anchors>
            <certificates src="user" />
        </trust-anchors>
    </debug-overrides>
</network-security-config>
`;

export const MANIFEST_ATTRIBUTE = `<application
    android:networkSecurityConfig="@xml/network_security_config"
    ... >`;

/** The adb commands the emulator tab runs, for people who prefer a terminal. */
export function emulatorCommands(port: number | null | undefined): { set: string; clear: string } {
  return {
    set: `adb shell settings put global http_proxy 10.0.2.2:${port ?? 9090}`,
    clear: "adb shell settings put global http_proxy :0",
  };
}

/** A data: URL for an SVG, so it renders as an image (no inline markup, no scripts). */
export function svgDataUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
