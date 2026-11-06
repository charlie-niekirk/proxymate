//! Cancelling reads (R4): `Backend` reads take an optional `AbortSignal`,
//! applied here once for both the Tauri and the mock backend.

import type { Backend } from "@/lib/backend";

/**
 * Runs `read`, rejecting with the signal's reason as soon as it aborts. An
 * IPC call can't be taken back, so an aborted read's answer is dropped; one
 * aborted before it starts is never sent.
 */
export function abortable<T>(signal: AbortSignal | undefined, read: () => Promise<T>): Promise<T> {
  if (!signal) return read();
  if (signal.aborted) return Promise.reject(abortReason(signal));
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => reject(abortReason(signal));
    signal.addEventListener("abort", onAbort, { once: true });
    read()
      .then(resolve, reject)
      .finally(() => signal.removeEventListener("abort", onAbort));
  });
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The read was aborted", "AbortError");
}

/** Where each read that takes a signal has it: the index of its trailing `signal` parameter. */
export const SIGNAL_ARG = {
  getFlow: 2,
  getFlowDetail: 2,
  getBody: 5,
  getWebsocketMessages: 5,
  getWebsocketPayload: 3,
  getWebsocketOriginal: 3,
  getSseEvents: 5,
  getBreakpointBody: 1,
  getMcpAudit: 2,
  getFlowViews: 3,
  decodeBody: 3,
  flowOperation: 2,
  getScriptConsole: 2,
} as const satisfies Partial<Record<keyof Backend, number>>;

/** `backend` with every `SIGNAL_ARG` read made abortable. */
export function withSignals(backend: Backend): Backend {
  const wrapped: Backend = { ...backend };
  for (const [name, at] of Object.entries(SIGNAL_ARG) as [keyof typeof SIGNAL_ARG, number][]) {
    const read = backend[name] as (...args: unknown[]) => Promise<unknown>;
    (wrapped as unknown as Record<string, unknown>)[name] = (...args: unknown[]) => {
      const signal = args[at] as AbortSignal | undefined;
      return abortable(signal, () => read.apply(backend, args.slice(0, at)));
    };
  }
  return wrapped;
}
