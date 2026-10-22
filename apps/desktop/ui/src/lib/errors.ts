import type { FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";

/** Something the user can do about a failed flow. */
export type FixAction = "cert-wizard" | "bypass" | "insecure-upstream" | "timeouts" | "tls-settings";

export interface FailureHelp {
  /** One line on what went wrong. */
  title: string;
  /** The engine's own message, which names the host and the detail. */
  detail: string;
  actions: FixAction[];
}

/**
 * Turns a failed flow's error into a headline and the fixes that apply:
 * a client refusing our certificate (CA not trusted, or pinning), an
 * upstream certificate we can't verify, an old TLS version, DNS and
 * connect failures, timeouts.
 */
export function explainFailure(flow: FlowSummary_Serialize, detail: FlowDetail_Serialize | null): FailureHelp | null {
  const error = detail?.error;
  if (!error) return null;
  const e = error.toLowerCase();
  if (flow.tls === "likely_pinned") {
    return {
      title: "The client refused Proxymate's certificate",
      detail: error,
      actions: ["cert-wizard", "bypass"],
    };
  }
  if (e.includes("certificate isn't trusted") || e.includes("unknownissuer") || e.includes("self-signed")) {
    return { title: "The server's certificate isn't trusted", detail: error, actions: ["insecure-upstream", "bypass"] };
  }
  if (e.includes("certificate has expired") || e.includes("isn't valid for that name")) {
    return { title: "The server's certificate is invalid", detail: error, actions: ["insecure-upstream", "bypass"] };
  }
  if (e.includes("protocol version") || e.includes("tls 1.0") || e.includes("tlsv1.0") || e.includes("handshake failure")) {
    return {
      title: "The server only speaks an old TLS version",
      detail: `${error} Proxymate supports TLS 1.2 and 1.3; add the host to the SSL bypass list to tunnel it instead.`,
      actions: ["bypass"],
    };
  }
  if (e.includes("tls handshake with") && flow.tls === "intercepted") {
    return { title: "The TLS handshake with the server failed", detail: error, actions: ["bypass", "tls-settings"] };
  }
  if (e.includes("can't resolve") || e.includes("failed to lookup") || e.includes("nodename nor servname")) {
    return { title: `Couldn't find ${flow.host}`, detail: `${error} Check the host name and this Mac's DNS.`, actions: [] };
  }
  if (e.includes("timed out")) {
    return { title: "The server didn't answer in time", detail: error, actions: ["timeouts"] };
  }
  if (e.includes("connection refused") || e.includes("can't reach")) {
    return { title: `Nothing answered at ${flow.host}:${flow.port}`, detail: `${error} Is the server running?`, actions: [] };
  }
  if (e.includes("client went away") || e.includes("client closed")) {
    return { title: "The client gave up", detail: error, actions: [] };
  }
  return { title: "The flow failed", detail: error, actions: [] };
}
