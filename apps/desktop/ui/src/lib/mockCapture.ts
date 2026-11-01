import type {
  AutoSaveSettings_Serialize,
  AutoSaveStatus_Serialize,
  InterceptApp,
  InterceptSession_Serialize,
  RecordingSettings_Serialize,
} from "@/bindings";
import type { Backend } from "@/lib/backend";

type CaptureMethods = Pick<
  Backend,
  | "listInterceptApps"
  | "listIntercepts"
  | "intercept"
  | "getRecordingSettings"
  | "setRecordingSettings"
  | "neverRecordHost"
  | "getAutosaveSettings"
  | "setAutosaveSettings"
  | "autosaveNow"
  | "chooseAutosaveFolder"
>;

const APPS: InterceptApp[] = [
  { target: "terminal", name: "Terminal", installed: true },
  { target: "iterm", name: "iTerm", installed: true },
  { target: "chrome", name: "Google Chrome", installed: true },
  { target: "edge", name: "Microsoft Edge", installed: false },
  { target: "brave", name: "Brave Browser", installed: false },
  { target: "arc", name: "Arc", installed: true },
  { target: "firefox", name: "Firefox", installed: true },
];

/** Like `capture::normalize_host` in Rust. */
export function normalizeHost(raw: string): string {
  let s = raw.trim();
  const scheme = s.indexOf("://");
  if (scheme >= 0) s = s.slice(scheme + 3);
  s = s.split(/[/?#]/)[0] ?? "";
  const at = s.lastIndexOf("@");
  if (at >= 0) s = s.slice(at + 1);
  if (s.startsWith("[")) s = s.slice(1, Math.max(1, s.indexOf("]")));
  else if (/^[^:]+:\d+$/.test(s)) s = s.slice(0, s.lastIndexOf(":"));
  return s.toLowerCase();
}

function normalizeList(list: string[]): string[] {
  const out: string[] = [];
  for (const raw of list) {
    const host = normalizeHost(raw);
    if (!host) continue;
    if (/\s/.test(host)) throw new Error(`"${raw}" isn't a host: hosts have no spaces`);
    if (!out.includes(host)) out.push(host);
  }
  return out;
}

/**
 * The mock backend's launchers (nothing starts), recording lists and
 * auto-save. `?unrecorded` seeds a never-recorded host and a count for the
 * footer; `?autosave` turns auto-save on with a last save.
 */
export function createMockCapture(changed: () => void) {
  const params = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : new URLSearchParams();
  let recording: RecordingSettings_Serialize = params.has("unrecorded")
    ? { only: [], never: ["*.apple.com", "telemetry.example.org"] }
    : { only: [], never: [] };
  let unrecorded = params.has("unrecorded") ? 128 : 0;
  let autosave: AutoSaveSettings_Serialize = params.has("autosave")
    ? { enabled: true, interval_minutes: 10, folder: "/Users/you/Documents/Proxymate", keep: 5 }
    : { enabled: false, interval_minutes: 10, folder: "", keep: 5 };
  let autosaveStatus: AutoSaveStatus_Serialize = {
    saving: false,
    last_at_ms: params.has("autosave") ? Date.now() - 4 * 60_000 : null,
    last_path: params.has("autosave") ? "/Users/you/Documents/Proxymate/Proxymate Autosave 2026-11-01T14-03-22Z.proxymate" : null,
    error: null,
  };
  let running: InterceptSession_Serialize[] = [];
  let nextId = 1;

  const methods: CaptureMethods = {
    async listInterceptApps() {
      return APPS;
    },
    async listIntercepts() {
      return running;
    },
    async intercept(target) {
      const app = APPS.find((a) => a.target === target);
      if (!app?.installed) throw new Error(`${app?.name ?? target} isn't installed in /Applications`);
      const session: InterceptSession_Serialize = {
        id: nextId++,
        target,
        name: app.name,
        pid: target === "terminal" || target === "iterm" ? null : 40_000 + nextId,
        started_at_ms: Date.now(),
        notes: target === "firefox" ? ["Firefox trusts the Proxymate CA only if the keychain does (mock)."] : [],
      };
      running = [...running, session];
      return session;
    },
    async getRecordingSettings() {
      return recording;
    },
    async setRecordingSettings(settings) {
      recording = { only: normalizeList(settings.only ?? []), never: normalizeList(settings.never ?? []) };
      return recording;
    },
    async neverRecordHost(host) {
      recording = { ...recording, never: normalizeList([...recording.never, host]) };
      unrecorded += 1;
      changed();
      return recording;
    },
    async getAutosaveSettings() {
      return autosave;
    },
    async setAutosaveSettings(settings) {
      const next = { ...autosave, ...settings, folder: (settings.folder ?? autosave.folder).trim() };
      if (next.interval_minutes < 1 || next.interval_minutes > 1440) throw new Error("save every 1 to 1,440 minutes");
      if (next.keep < 1 || next.keep > 1000) throw new Error("keep 1 to 1,000 saved sessions");
      if (next.enabled && !next.folder.startsWith("/")) throw new Error("choose a folder to save into");
      autosave = next;
      return autosave;
    },
    async autosaveNow() {
      if (!autosave.folder) throw new Error("choose a folder to save into");
      autosaveStatus = {
        saving: false,
        last_at_ms: Date.now(),
        last_path: `${autosave.folder}/Proxymate Autosave ${new Date().toISOString().slice(0, 19).replaceAll(":", "-")}Z.proxymate`,
        error: null,
      };
      changed();
      return 0;
    },
    async chooseAutosaveFolder() {
      return "/Users/you/Documents/Proxymate";
    },
  };
  return {
    methods,
    status: () => ({ unrecorded, autosave: autosaveStatus }),
  };
}
