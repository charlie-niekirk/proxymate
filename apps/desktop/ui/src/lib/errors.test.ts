import { describe, expect, it } from "vitest";

import type { ErrorKind, FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";

import { explainFailure } from "./errors";

const flow = (tls: FlowSummary_Serialize["tls"]) => ({ host: "api.example.com", port: 443, tls }) as FlowSummary_Serialize;
const detail = (error: string | null, error_kind: ErrorKind | null = null) =>
  ({ error, error_kind }) as FlowDetail_Serialize;

describe("explainFailure", () => {
  it("offers the wizard and the bypass for a pinning client", () => {
    const help = explainFailure(
      flow("likely_pinned"),
      detail("The client rejected the Proxymate certificate. It sent an unknown_ca alert.", "client_rejected_cert"),
    );
    expect(help?.actions).toEqual(["cert-wizard", "bypass"]);
  });

  it("offers insecure upstream for an untrusted server certificate", () => {
    const help = explainFailure(
      flow("intercepted"),
      detail("api.example.com's certificate isn't trusted: it is self-signed", "upstream_cert_untrusted"),
    );
    expect(help?.title).toMatch(/isn't trusted/);
    expect(help?.actions).toContain("insecure-upstream");
  });

  it("points client certificate and upstream proxy failures at Settings › Network", () => {
    const cert = explainFailure(
      flow("intercepted"),
      detail("api.example.com asks for a client certificate. Add one…", "client_cert_required"),
    );
    expect(cert?.title).toBe("The server wants a client certificate");
    expect(cert?.actions).toEqual(["network-settings"]);
    const auth = explainFailure(
      flow("intercepted"),
      detail(
        "the upstream proxy http://127.0.0.1:3128 rejected the username and password (407). Check them in Settings › Network.",
        "upstream_proxy_auth",
      ),
    );
    expect(auth?.title).toBe("The upstream proxy wants other credentials");
    const unreachable = explainFailure(
      flow("none"),
      detail("the upstream proxy http://p:1: can't reach p:1: Connection refused", "upstream_proxy"),
    );
    expect(unreachable?.title).toBe("The upstream proxy couldn't connect");
    expect(unreachable?.actions).toEqual(["network-settings"]);
  });

  it("names DNS, refused connections and timeouts", () => {
    expect(explainFailure(flow("none"), detail("can't resolve nope.test: failed to lookup address", "dns"))?.title).toBe(
      "Couldn't find api.example.com",
    );
    expect(explainFailure(flow("none"), detail("can't reach api.example.com:443: Connection refused", "refused"))?.title).toMatch(
      /Nothing answered/,
    );
    expect(explainFailure(flow("none"), detail("connecting to api.example.com:443 timed out", "timeout"))?.actions).toEqual([
      "timeouts",
    ]);
    expect(explainFailure(flow("none"), detail(null))).toBeNull();
  });

  it("goes by the kind, not the wording", () => {
    // A message that reads like a timeout but isn't one.
    const help = explainFailure(flow("none"), detail("the upstream said: request timed out", "upstream"));
    expect(help?.title).toBe("The flow failed");
    // No kind (a detail from before the engine recorded kinds): generic.
    expect(explainFailure(flow("none"), detail("connecting to x timed out"))?.title).toBe("The flow failed");
  });
});
