import type { InterceptTarget } from "@/bindings";

/** The Intercept menu, in order (`InterceptTarget::ALL`); names match the engine's. */
export const INTERCEPT_TARGETS: { target: InterceptTarget; name: string; terminal: boolean }[] = [
  { target: "terminal", name: "Terminal", terminal: true },
  { target: "iterm", name: "iTerm", terminal: true },
  { target: "chrome", name: "Google Chrome", terminal: false },
  { target: "edge", name: "Microsoft Edge", terminal: false },
  { target: "brave", name: "Brave Browser", terminal: false },
  { target: "arc", name: "Arc", terminal: false },
  { target: "firefox", name: "Firefox", terminal: false },
];
