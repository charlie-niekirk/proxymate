import type {
  ClientIdentity_Serialize,
  ListenerStatus_Serialize,
  NetworkSettings_Deserialize,
  NetworkSettings_Serialize,
  ProxyServer_Deserialize,
  ProxyServer_Serialize,
  UpstreamProxy_Deserialize,
  UpstreamProxy_Serialize,
} from "@/bindings";
import type { Backend } from "@/lib/backend";

type NetworkMethods = Pick<Backend, "getNetworkSettings" | "setNetworkSettings">;

const MAIN = "127.0.0.1:9090";

/** The defaults, plus one example of each so every section has something to show. */
function initial(): NetworkSettings_Serialize {
  return {
    upstream: {
      rules: [
        {
          enabled: true,
          hosts: ["*.corp.example"],
          proxy: { kind: "http", host: "proxy.corp.example", port: 3128, username: "me", password: null, has_password: true },
        },
      ],
      bypass: ["localhost", "127.0.0.1", "::1", "*.local"],
      use_system_previous: false,
    },
    reverse_proxies: [{ enabled: true, listen_port: 8443, target_url: "http://localhost:3000", preserve_host: false, tls: true }],
    client_certs: [],
    dns: [{ enabled: true, host: "api.staging.test", ip: "127.0.0.1" }],
  };
}

function server(s: ProxyServer_Deserialize, saved: boolean): ProxyServer_Serialize {
  const password = s.password ?? null;
  return {
    host: (s.host ?? "").trim(),
    port: s.port ?? 0,
    username: s.username || null,
    password: null,
    has_password: password === null ? (s.has_password ?? saved) : password !== "",
  };
}

function proxy(p: UpstreamProxy_Deserialize): UpstreamProxy_Serialize {
  if (p.kind === "direct") return p;
  return { kind: p.kind, ...server(p, false) };
}

/**
 * Settings › Network in the browser mock: kept in memory, with the
 * engine's main checks (the loop guard, ports, URLs, IPs), and the reverse
 * proxies reported as listeners.
 */
export function createMockNetwork(): NetworkMethods & { listeners(): ListenerStatus_Serialize[] } {
  let settings = initial();
  return {
    listeners: () => [
      { kind: "socks5", addr: MAIN, target: null, error: null },
      ...settings.reverse_proxies
        .filter((r) => r.enabled)
        .map(
          (r): ListenerStatus_Serialize => ({
            kind: r.tls ? "reverse_tls" : "reverse",
            addr: r.listen_port === 8080 ? null : `127.0.0.1:${r.listen_port}`,
            target: r.target_url,
            error: r.listen_port === 8080 ? "port 8080 is in use" : null,
          }),
        ),
    ],
    async getNetworkSettings() {
      return settings;
    },
    async setNetworkSettings(next: NetworkSettings_Deserialize) {
      const rules = (next.upstream?.rules ?? []).map((r, i) => {
        const p = proxy(r.proxy);
        if (p.kind !== "direct") {
          if (!p.host || !p.port) throw new Error(`upstream rule ${i + 1} needs a proxy host and port`);
          if (["localhost", "127.0.0.1"].includes(p.host) && p.port === 9090) {
            throw new Error(`upstream rule ${i + 1} points at Proxymate itself (${p.host}:9090), which would loop`);
          }
        }
        if (r.enabled && r.hosts.every((h) => !h.trim())) {
          throw new Error(`upstream rule ${i + 1} matches no hosts (use * for every host)`);
        }
        return { ...r, proxy: p };
      });
      for (const r of next.reverse_proxies ?? []) {
        if (r.listen_port < 1024) throw new Error(`reverse proxy port ${r.listen_port}: pick one from 1024 up`);
        if (r.listen_port === 9090) throw new Error("port 9090 is the proxy's own port; pick another for the reverse proxy");
        if (!/^https?:\/\/[^/]+/.test(r.target_url.trim())) {
          throw new Error(`the reverse proxy target "${r.target_url}" must be an http:// or https:// URL`);
        }
      }
      for (const d of next.dns ?? []) {
        if (!d.host.trim()) throw new Error("a DNS override needs a host");
        if (!/^[\d.]+$|:/.test(d.ip.trim())) throw new Error(`"${d.ip}" isn't an IP address`);
      }
      const certs = (next.client_certs ?? []).map((c) => {
        const identity: ClientIdentity_Serialize =
          c.identity.kind === "pem"
            ? c.identity
            : {
                kind: "pkcs12",
                path: c.identity.path,
                password: null,
                has_password: c.identity.password == null ? (c.identity.has_password ?? false) : c.identity.password !== "",
              };
        return { ...c, identity };
      });
      settings = {
        upstream: {
          rules,
          bypass: next.upstream?.bypass ?? [],
          use_system_previous: next.upstream?.use_system_previous ?? false,
        },
        reverse_proxies: next.reverse_proxies ?? [],
        client_certs: certs,
        dns: next.dns ?? [],
      };
      return settings;
    },
  };
}
