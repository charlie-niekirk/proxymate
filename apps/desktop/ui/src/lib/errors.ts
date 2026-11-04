import type { FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";

/** Something the user can do about a failed flow. */
export type FixAction = "cert-wizard" | "bypass" | "insecure-upstream" | "timeouts" | "tls-settings" | "network-settings";

export interface FailureHelp {
  /** One line on what went wrong. */
  title: string;
  /** The engine's own message, which names the host and the detail. */
  detail: string;
  actions: FixAction[];
}

/**
 * Turns a failed flow's error into a headline and the fixes that apply,
 * by its kind (R3; the engine records it where the failure happens, and
 * infers it for sessions saved before format 8): a client refusing our
 * certificate (CA not trusted, or pinning), an upstream certificate we
 * can't verify, an old TLS version, a server that wants a client
 * certificate, an upstream proxy that refused or would loop (M18), DNS and
 * connect failures, timeouts. The message is shown as it is.
 */
export function explainFailure(flow: FlowSummary_Serialize, detail: FlowDetail_Serialize | null): FailureHelp | null {
  const error = detail?.error;
  if (!error) return null;
  const help = (title: string, actions: FixAction[] = [], text = error): FailureHelp => ({ title, detail: text, actions });
  if (flow.tls === "likely_pinned") {
    return help("The client refused Proxymate's certificate", ["cert-wizard", "bypass"]);
  }
  switch (detail.error_kind ?? "other") {
    case "client_rejected_cert":
      return help("The client refused Proxymate's certificate", ["cert-wizard", "bypass"]);
    case "client_tls":
      return help("The TLS handshake with the client failed", ["cert-wizard", "bypass"]);
    case "client_cert_required":
      return help("The server wants a client certificate", ["network-settings"]);
    case "upstream_proxy_auth":
      return help("The upstream proxy wants other credentials", ["network-settings"]);
    case "upstream_proxy_loop":
      return help("The upstream proxy is Proxymate itself", ["network-settings"]);
    case "upstream_proxy":
      return help("The upstream proxy couldn't connect", ["network-settings"]);
    case "upstream_cert_untrusted":
      return help("The server's certificate isn't trusted", ["insecure-upstream", "bypass"]);
    case "upstream_cert_invalid":
      return help("The server's certificate is invalid", ["insecure-upstream", "bypass"]);
    case "upstream_tls_version":
      return help(
        "The server only speaks an old TLS version",
        ["bypass"],
        `${error} Proxymate supports TLS 1.2 and 1.3; add the host to the SSL bypass list to tunnel it instead.`,
      );
    case "upstream_tls":
      return help("The TLS handshake with the server failed", ["bypass", "tls-settings"]);
    case "dns":
      return help(`Couldn't find ${flow.host}`, [], `${error} Check the host name and this Mac's DNS.`);
    case "timeout":
      return help("The server didn't answer in time", ["timeouts"]);
    case "refused":
      return help(`Nothing answered at ${flow.host}:${flow.port}`, [], `${error} Is the server running?`);
    case "client_gone":
      return help("The client gave up");
    case "upstream":
    case "other":
      return help("The flow failed");
  }
}
