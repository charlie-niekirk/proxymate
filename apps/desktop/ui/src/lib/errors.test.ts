import { describe, expect, it } from "vitest";

import type { FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";

import { explainFailure } from "./errors";

const flow = (tls: FlowSummary_Serialize["tls"]) => ({ host: "api.example.com", port: 443, tls }) as FlowSummary_Serialize;
const detail = (error: string | null) => ({ error }) as FlowDetail_Serialize;

describe("explainFailure", () => {
  it("offers the wizard and the bypass for a pinning client", () => {
    const help = explainFailure(flow("likely_pinned"), detail("The client rejected the Proxymate certificate. It sent an unknown_ca alert."));
    expect(help?.actions).toEqual(["cert-wizard", "bypass"]);
  });

  it("offers insecure upstream for an untrusted server certificate", () => {
    const help = explainFailure(flow("intercepted"), detail("api.example.com's certificate isn't trusted: it is self-signed"));
    expect(help?.title).toMatch(/isn't trusted/);
    expect(help?.actions).toContain("insecure-upstream");
  });

  it("names DNS, refused connections and timeouts", () => {
    expect(explainFailure(flow("none"), detail("can't resolve nope.test: failed to lookup address"))?.title).toBe("Couldn't find api.example.com");
    expect(explainFailure(flow("none"), detail("can't reach api.example.com:443: Connection refused"))?.title).toMatch(/Nothing answered/);
    expect(explainFailure(flow("none"), detail("connecting to api.example.com:443 timed out"))?.actions).toEqual(["timeouts"]);
    expect(explainFailure(flow("none"), detail(null))).toBeNull();
  });
});
