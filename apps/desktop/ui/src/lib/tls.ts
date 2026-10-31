import type { CertSummary, Timing_Serialize, TlsStatus, Via_Serialize } from "@/bindings";

const SSL_TITLE: Record<TlsStatus, string> = {
  none: "",
  intercepted: "Decrypted by Proxymate",
  passthrough: "Not decrypted: SSL passthrough",
  likely_pinned: "The client rejected the Proxymate certificate (likely pinned)",
};

/** Tooltip of the SSL column. */
export function sslTitle(tls: TlsStatus): string {
  return SSL_TITLE[tls];
}

export interface Phase {
  label: string;
  ms: number;
  /** Time before this phase starts, from the first phase. */
  offset: number;
  /** CSS variable colouring the bar. */
  token: string;
}

/** The phases that have a value, in the order they happen. */
export function timingPhases(t: Timing_Serialize): Phase[] {
  const all: [string, number | null, string][] = [
    ["DNS", t.dns_ms, "--timing-dns"],
    ["Connect", t.connect_ms, "--timing-connect"],
    ["TLS", t.tls_ms, "--timing-tls"],
    ["Waiting (TTFB)", t.ttfb_ms, "--timing-wait"],
    ["Download", t.transfer_ms, "--timing-receive"],
  ];
  const phases: Phase[] = [];
  let offset = 0;
  for (const [label, ms, token] of all) {
    if (ms === null) continue;
    phases.push({ label, ms, offset, token });
    offset += ms;
  }
  return phases;
}

/** True if `cert` had expired at `atMs` (e.g. when the flow was captured). */
export function certExpiredAt(cert: CertSummary, atMs: number): boolean {
  const notAfter = Date.parse(cert.not_after);
  return !Number.isNaN(notAfter) && notAfter < atMs;
}

/** How the upstream connection was made (M18), as label/value rows. */
export function routeRows(via: Via_Serialize | null): [string, string][] {
  if (!via) return [];
  const rows: [string, string][] = [["Route", via.proxy ? `Through ${via.proxy}` : "Direct"]];
  if (via.addr) rows.push(["Connected to", via.addr]);
  if (via.dns_override) rows.push(["DNS override", via.dns_override]);
  if (via.reverse_port != null) rows.push(["Arrived on", `Reverse proxy :${via.reverse_port}`]);
  return rows;
}
