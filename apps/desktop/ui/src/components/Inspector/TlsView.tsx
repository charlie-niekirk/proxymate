import type { CertSummary, FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";
import { KvTable, Placeholder, type Kv } from "@/components/Inspector/KvTable";
import { certExpiredAt } from "@/lib/tls";

/** The TLS tab: how both legs were negotiated, and the upstream's chain. */
export function TlsView({ flow, detail }: { flow: FlowSummary_Serialize; detail: FlowDetail_Serialize }) {
  if (flow.tls === "passthrough") {
    return <Placeholder text="Not decrypted: this host is in the SSL exclude or bypass list, so the tunnel was relayed as is." />;
  }
  const tls = detail.tls;
  if (!tls) return <Placeholder text="Plain HTTP: no TLS" />;

  const handshakeFailed = tls.version === "";
  const verification = handshakeFailed
    ? "Failed (see the error)"
    : tls.upstream_verified
      ? "Verified"
      : "Not verified: insecure upstream allowed for this host";
  const rows: Kv = [
    ["Client SNI", tls.sni ?? "— (no SNI: IP address)"],
    ["Client offered ALPN", tls.client_alpn.length ? tls.client_alpn.join(", ") : "—"],
    ["Negotiated ALPN", tls.alpn ?? "— (none: HTTP/1.1)"],
    ["Upstream TLS version", tls.version || "—"],
    ["Upstream cipher", tls.cipher || "—"],
    ["Upstream certificate", verification],
  ];
  return (
    <div className="flex flex-col" data-testid="tls-view">
      <SectionTitle>Connection</SectionTitle>
      <KvTable rows={rows} />
      <SectionTitle>Upstream certificate chain</SectionTitle>
      {tls.chain.length === 0 ? (
        <Placeholder text="The upstream presented no certificate" />
      ) : (
        tls.chain.map((cert, i) => (
          <CertCard key={`${cert.sha256}-${i}`} cert={cert} index={i} capturedAt={flow.started_at_ms ?? 0} />
        ))
      )}
    </div>
  );
}

function SectionTitle({ children }: { children: string }) {
  return (
    <div className="text-muted-foreground bg-pane-header border-b px-2 py-1 text-[11px] font-semibold tracking-wide uppercase">
      {children}
    </div>
  );
}

function CertCard({ cert, index, capturedAt }: { cert: CertSummary; index: number; capturedAt: number }) {
  const expired = certExpiredAt(cert, capturedAt);
  const rows: Kv = [
    ["Issuer", cert.issuer],
    ["Valid", `${formatDate(cert.not_before)} → ${formatDate(cert.not_after)}${expired ? " (expired when captured)" : ""}`],
  ];
  if (cert.san.length) rows.push(["Names", cert.san.join(", ")]);
  rows.push(["Serial", cert.serial], ["SHA-256", cert.sha256]);
  return (
    <div className="border-b">
      <div className="flex items-baseline gap-2 px-2 pt-1.5 text-xs" data-selectable>
        <span className="text-muted-foreground font-mono tabular-nums">{index}</span>
        <span className="font-medium break-all">{cert.subject}</span>
        {index === 0 && <span className="text-muted-foreground">leaf</span>}
      </div>
      <KvTable rows={rows} />
    </div>
  );
}

function formatDate(rfc3339: string): string {
  if (!rfc3339) return "—";
  return rfc3339.replace("T", " ").replace(/:\d\d(\.\d+)?Z$/, " UTC");
}
