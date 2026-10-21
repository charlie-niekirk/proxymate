import { describe, expect, it } from "vitest";

import type { DeviceClient_Deserialize } from "@/bindings";
import { activeDevices, emulatorCommands, progress, splitProxy, svgDataUrl } from "@/lib/devices";

const device = (ip: string, patch: Partial<DeviceClient_Deserialize>): DeviceClient_Deserialize => ({
  ip,
  nickname: null,
  decision: "allowed",
  label: ip,
  flows: 0,
  https_decrypted: 0,
  https_refused: 0,
  onboarding: false,
  last_seen_ms: null,
  waiting: 0,
  ...patch,
});

describe("device setup helpers", () => {
  it("reports progress from flow counts", () => {
    expect(progress({ flows: 0, https_decrypted: 0, https_refused: 0 })).toBe("waiting");
    expect(progress({ flows: 3, https_decrypted: 0, https_refused: 0 })).toBe("connected");
    expect(progress({ flows: 3, https_decrypted: 0, https_refused: 2 })).toBe("untrusted");
    expect(progress({ flows: 3, https_decrypted: 1, https_refused: 2 })).toBe("decrypting");
  });

  it("lists allowed devices that showed up, newest first", () => {
    const list = activeDevices([
      device("10.0.0.2", { flows: 1, last_seen_ms: 10 }),
      device("10.0.0.3", { decision: "pending", flows: 4 }),
      device("10.0.0.4", { onboarding: true, last_seen_ms: 20 }),
      device("10.0.0.5", {}),
    ]);
    expect(list.map((d) => d.ip)).toEqual(["10.0.0.4", "10.0.0.2"]);
  });

  it("splits the proxy address for the Wi-Fi fields", () => {
    expect(splitProxy("192.168.1.20:9090")).toEqual({ host: "192.168.1.20", port: "9090" });
    expect(splitProxy(null)).toBeNull();
    expect(splitProxy("nope")).toBeNull();
  });

  it("builds emulator commands and SVG data URLs", () => {
    expect(emulatorCommands(9091).set).toBe("adb shell settings put global http_proxy 10.0.2.2:9091");
    expect(svgDataUrl('<svg a="1"/>')).toBe("data:image/svg+xml;charset=utf-8,%3Csvg%20a%3D%221%22%2F%3E");
  });
});
