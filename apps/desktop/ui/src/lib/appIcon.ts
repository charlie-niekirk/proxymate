import { convertFileSrc, isTauri } from "@tauri-apps/api/core";

/** The `pmicon://` URL of a client app's icon; null outside the app. */
export function appIconUrl(name: string): string | null {
  return isTauri() ? convertFileSrc(name, "pmicon") : null;
}
