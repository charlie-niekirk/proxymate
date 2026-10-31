import { describe, expect, it } from "vitest";

import { certExpiredAt, routeRows, sslTitle, timingPhases } from "./tls";

describe("timingPhases", () => {
  it("keeps recorded phases in order with running offsets", () => {
    const phases = timingPhases({
      dns_ms: 2,
      connect_ms: null,
      tls_ms: 10,
      ttfb_ms: 30,
      transfer_ms: 5,
      reused_connection: false,
    });
    expect(phases.map((p) => [p.label, p.offset, p.ms])).toEqual([
      ["DNS", 0, 2],
      ["TLS", 2, 10],
      ["Waiting (TTFB)", 12, 30],
      ["Download", 42, 5],
    ]);
  });

  it("is empty when nothing was timed", () => {
    expect(
      timingPhases({ dns_ms: null, connect_ms: null, tls_ms: null, ttfb_ms: null, transfer_ms: null, reused_connection: true }),
    ).toEqual([]);
  });
});

describe("certificates and the SSL column", () => {
  it("judges expiry at capture time", () => {
    const cert = { subject: "", issuer: "", serial: "", not_before: "", not_after: "2026-01-01T00:00:00Z", san: [], sha256: "" };
    expect(certExpiredAt(cert, Date.parse("2025-12-31T00:00:00Z"))).toBe(false);
    expect(certExpiredAt(cert, Date.parse("2026-01-02T00:00:00Z"))).toBe(true);
  });

  it("explains each TLS status", () => {
    expect(sslTitle("none")).toBe("");
    expect(sslTitle("passthrough")).toContain("Not decrypted");
  });
});

describe("routeRows", () => {
  it("names the upstream proxy, the address, overrides and the reverse port", () => {
    expect(routeRows(null)).toEqual([]);
    expect(
      routeRows({ proxy: null, addr: "93.184.215.14:443", dns_override: null, client_cert: null, reverse_port: null }),
    ).toEqual([
      ["Route", "Direct"],
      ["Connected to", "93.184.215.14:443"],
    ]);
    expect(
      routeRows({
        proxy: "socks5://127.0.0.1:1080",
        addr: "127.0.0.1:1080",
        dns_override: "*.test → 127.0.0.1",
        client_cert: "CN=me",
        reverse_port: 8443,
      }),
    ).toEqual([
      ["Route", "Through socks5://127.0.0.1:1080"],
      ["Connected to", "127.0.0.1:1080"],
      ["DNS override", "*.test → 127.0.0.1"],
      ["Arrived on", "Reverse proxy :8443"],
    ]);
  });
});
