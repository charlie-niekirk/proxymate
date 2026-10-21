import type {
  AndroidStatus_Serialize,
  DeviceAccess_Serialize,
  DevicesSnapshot_Deserialize,
  DevicesSnapshot_Serialize,
} from "@/bindings";
import type { Backend } from "@/lib/backend";

type DeviceMethods = Pick<
  Backend,
  | "getDeviceAccess"
  | "setLanAccess"
  | "setLanInterface"
  | "subscribeDevices"
  | "decideDevice"
  | "forgetDevice"
  | "getSimulatorStatus"
  | "installSimulatorCert"
  | "getAndroidStatus"
  | "setAndroidProxy"
  | "pushAndroidCert"
>;

/** A stand-in QR code: a static pattern, not a scannable code. */
const MOCK_QR = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21" width="200" height="200" shape-rendering="crispEdges"><rect width="21" height="21" fill="#fff"/><path fill="#000" d="${Array.from(
  { length: 21 * 21 },
  (_, i) => i,
)
  .filter((i) => (i * 7919) % 5 < 2)
  .map((i) => `M${i % 21} ${Math.floor(i / 21)}h1v1h-1z`)
  .join("")}"/></svg>`;

/**
 * Device setup in the browser mock: two LAN interfaces, nothing connected.
 * With `pendingIp`, that LAN client is waiting for an answer (the allow prompt).
 */
export function createMockDevices(port: number, pendingIp: string | null = null): DeviceMethods {
  const interfaces = [
    { name: "en0", ip: "192.168.1.20" },
    { name: "bridge100", ip: "192.168.64.1" },
  ];
  let enabled = false;
  let chosen = "en0";
  let snapshot: DevicesSnapshot_Serialize = {
    devices: pendingIp
      ? [
          {
            ip: pendingIp,
            nickname: null,
            decision: "pending",
            label: pendingIp,
            flows: 0,
            https_decrypted: 0,
            https_refused: 0,
            onboarding: false,
            last_seen_ms: null,
            waiting: 2,
          },
        ]
      : [],
    simulator: { flows: 0, https_decrypted: 0, https_refused: 0 },
    emulator: { flows: 0, https_decrypted: 0, https_refused: 0 },
  };
  let onDevices: ((s: DevicesSnapshot_Deserialize) => void) | null = null;
  const push = () => onDevices?.(snapshot);
  let emulatorProxy: string | null = null;

  const access = (): DeviceAccess_Serialize => {
    const ip = interfaces.find((i) => i.name === chosen)?.ip ?? null;
    const proxy = ip ? `${ip}:${port}` : null;
    return {
      enabled,
      interface: chosen,
      interfaces,
      port,
      proxy,
      setup_url: enabled && proxy ? `http://${proxy}/` : null,
      qr_svg: enabled ? MOCK_QR : null,
      error: null,
    };
  };
  const android = (): AndroidStatus_Serialize => ({
    available: true,
    detail: null,
    devices: [{ serial: "emulator-5554", model: "sdk gphone64 arm64", emulator: true, state: "device", proxy: emulatorProxy }],
    emulator_proxy: `10.0.2.2:${port}`,
  });

  return {
    async getDeviceAccess() {
      return access();
    },
    async setLanAccess(on) {
      enabled = on;
      return access();
    },
    async setLanInterface(name) {
      chosen = name;
      return access();
    },
    async subscribeDevices(cb) {
      onDevices = cb;
      setTimeout(push, 0);
    },
    async decideDevice(ip, allow, nickname) {
      snapshot = {
        ...snapshot,
        devices: snapshot.devices.map((d) =>
          d.ip === ip
            ? {
                ...d,
                decision: allow ? "allowed" : "denied",
                waiting: 0,
                nickname,
                label: nickname ? `${nickname} (${ip})` : ip,
              }
            : d,
        ),
      };
      push();
      return snapshot;
    },
    async forgetDevice(ip) {
      snapshot = { ...snapshot, devices: snapshot.devices.filter((d) => d.ip !== ip) };
      push();
      return snapshot;
    },
    async getSimulatorStatus() {
      return {
        available: true,
        detail: null,
        booted: [{ udid: "MOCK-UDID", name: "iPhone 17 Pro", runtime: "iOS 26.2", cert_installed: false }],
      };
    },
    async installSimulatorCert() {
      return [{ udid: "MOCK-UDID", name: "iPhone 17 Pro", error: null }];
    },
    async getAndroidStatus() {
      return android();
    },
    async setAndroidProxy(_serial, on) {
      emulatorProxy = on ? `10.0.2.2:${port}` : null;
      return android();
    },
    async pushAndroidCert() {
      return "/sdcard/Download/proxymate-ca.crt";
    },
  };
}
