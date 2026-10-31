import { useQuery } from "@tanstack/react-query";
import { Plus, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";

import type {
  ClientCert_Deserialize,
  DnsOverride,
  ListenerStatus_Serialize,
  NetworkSettings_Deserialize,
  NetworkSettings_Serialize,
  ProxyServer_Deserialize,
  ReverseProxy,
  UpstreamProxy_Deserialize,
  UpstreamRule_Deserialize,
} from "@/bindings";
import { CopyField } from "@/components/DeviceSetup/parts";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { networkSettingsKey, proxyStatusKey, queryClient } from "@/lib/query";
import { cn } from "@/lib/utils";

import { ListEditor, Page, Section } from "./parts";

/**
 * Settings › Network (M18): upstream proxies, the SOCKS5 listener, reverse
 * proxies, client certificates and DNS overrides. Edits stay a draft until
 * Save, which checks and applies the whole page at once: passwords go to
 * the keychain and are never shown again, only "saved".
 */
export function NetworkPage() {
  const { data: settings } = useQuery({ queryKey: networkSettingsKey, queryFn: () => backend.getNetworkSettings() });
  if (!settings) return null;
  // Remounted on every save, so the draft starts from what the engine kept.
  return <NetworkForm key={JSON.stringify(settings)} saved={settings} />;
}

function NetworkForm({ saved }: { saved: NetworkSettings_Serialize }) {
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const [draft, setDraft] = useState<NetworkSettings_Deserialize>(() => structuredClone(saved));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const upstream = draft.upstream ?? { rules: [], bypass: [], use_system_previous: false };
  const rules = upstream.rules ?? [];
  const reverse = draft.reverse_proxies ?? [];
  const certs = draft.client_certs ?? [];
  const dns = draft.dns ?? [];
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const set = (patch: Partial<NetworkSettings_Deserialize>) => setDraft((d) => ({ ...d, ...patch }));
  const setUpstream = (patch: Partial<typeof upstream>) => set({ upstream: { ...upstream, ...patch } });
  const save = async () => {
    setError(null);
    setBusy(true);
    try {
      queryClient.setQueryData(networkSettingsKey, await backend.setNetworkSettings(draft));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      await queryClient.invalidateQueries({ queryKey: proxyStatusKey });
    }
  };
  const socks = status?.listeners.find((l) => l.kind === "socks5")?.addr ?? status?.listening ?? null;

  return (
    <Page testId="settings-network">
      <div className="bg-background sticky -top-4 z-10 -mx-4 -mt-4 flex items-center gap-2 border-b px-4 py-2">
        {error ? (
          <p className="min-w-0 flex-1 text-[var(--s-failed)]" role="alert" data-testid="settings-error">
            {error}
          </p>
        ) : (
          <span className="text-muted-foreground min-w-0 flex-1">
            {dirty ? "Unsaved changes" : "New connections use these settings straight away."}
          </span>
        )}
        <Button variant="ghost" size="sm" disabled={!dirty || busy} onClick={() => setDraft(structuredClone(saved))}>
          Revert
        </Button>
        <Button size="sm" disabled={!dirty || busy} onClick={() => void save()} data-testid="network-save">
          {busy ? "Saving…" : "Save"}
        </Button>
      </div>

      <Section
        title="Upstream proxies"
        description="Send Proxymate's own connections through another proxy, e.g. a corporate one. The first enabled rule whose hosts match wins; HTTPS and WebSockets go through CONNECT, plain HTTP in absolute form, and SOCKS5 proxies resolve names themselves."
        testId="network-upstream"
      >
        <Rows
          items={rules}
          empty="Every connection goes directly to its server"
          onRemove={(i) => setUpstream({ rules: rules.filter((_, j) => j !== i) })}
          render={(rule, i) => (
            <UpstreamRow
              rule={rule}
              onChange={(next) => setUpstream({ rules: rules.map((r, j) => (j === i ? next : r)) })}
              index={i}
            />
          )}
        />
        <AddButton
          onClick={() =>
            setUpstream({
              rules: [...rules, { enabled: true, hosts: ["*"], proxy: { kind: "http", host: "", port: 3128 } }],
            })
          }
          testId="network-add-upstream"
        >
          Add upstream rule
        </AddButton>
        <div className="grid grid-cols-[180px_1fr] items-start gap-3">
          <span className="pt-1">Always direct</span>
          <ListEditor
            items={upstream.bypass ?? []}
            onChange={(bypass) => setUpstream({ bypass })}
            placeholder="*.internal.example"
            empty="Nothing bypasses the upstream rules"
            testId="network-bypass"
          />
        </div>
        <label className="flex items-start gap-3">
          <Switch
            checked={upstream.use_system_previous ?? false}
            onCheckedChange={(on) => setUpstream({ use_system_previous: on })}
            aria-label="Use the proxy that was set before Proxymate"
            data-testid="network-previous"
          />
          <span className="leading-4">
            Use the proxy that was set before Proxymate for hosts no rule matches
            <span className="text-muted-foreground block">
              While Proxymate is the system proxy, that's the one it replaced (with its bypass list); otherwise the
              system proxy itself.
            </span>
          </span>
        </label>
      </Section>

      <Section
        title="SOCKS5"
        description="The proxy port also speaks SOCKS5 (no authentication). Connections take the same path as CONNECT: the SSL lists decide what is decrypted, plain HTTP is recorded as HTTP and anything else as a tunnel."
        testId="network-socks"
      >
        {socks && (
          <>
            <CopyField value={`socks5h://${socks}`} label="Copy SOCKS5 address" testId="network-socks-address" />
            <CopyField
              value={`curl --socks5-hostname ${socks} https://example.com/`}
              label="Copy curl example"
              testId="network-socks-curl"
            />
          </>
        )}
      </Section>

      <Section
        title="Reverse proxies"
        description="Listen on a local port and forward everything to one server, recorded like proxied traffic. Point an app at http://localhost:<port> when it can't use a proxy. With HTTPS the port serves a Proxymate certificate for localhost."
        testId="network-reverse"
      >
        <Rows
          items={reverse}
          empty="No reverse proxies"
          onRemove={(i) => set({ reverse_proxies: reverse.filter((_, j) => j !== i) })}
          render={(r, i) => (
            <ReverseRow
              value={r}
              listener={status?.listeners.find((l) => l.kind !== "socks5" && l.addr?.endsWith(`:${r.listen_port}`))}
              failed={status?.listeners.find((l) => l.kind !== "socks5" && !l.addr && l.target === r.target_url)}
              onChange={(next) => set({ reverse_proxies: reverse.map((x, j) => (j === i ? next : x)) })}
            />
          )}
        />
        <AddButton
          onClick={() =>
            set({
              reverse_proxies: [
                ...reverse,
                { enabled: true, listen_port: 8080, target_url: "http://localhost:3000", preserve_host: false, tls: false },
              ],
            })
          }
          testId="network-add-reverse"
        >
          Add reverse proxy
        </AddButton>
      </Section>

      <Section
        title="Client certificates"
        description="Presented to servers that ask for one (mutual TLS). A PKCS#12 file's password is kept in the keychain; a PEM key must be unencrypted."
        testId="network-certs"
      >
        <Rows
          items={certs}
          empty="No client certificates"
          onRemove={(i) => set({ client_certs: certs.filter((_, j) => j !== i) })}
          render={(c, i) => (
            <CertRow value={c} onChange={(next) => set({ client_certs: certs.map((x, j) => (j === i ? next : x)) })} />
          )}
        />
        <AddButton
          onClick={() =>
            set({
              client_certs: [...certs, { enabled: true, hosts: [""], identity: { kind: "pkcs12", path: "", password: null } }],
            })
          }
          testId="network-add-cert"
        >
          Add client certificate
        </AddButton>
      </Section>

      <Section
        title="DNS overrides"
        description="Connect to an address of your choosing for matching hosts, before any lookup, like /etc/hosts for Proxymate alone. The server name (SNI) and Host header don't change."
        testId="network-dns"
      >
        <Rows
          items={dns}
          empty="Names resolve normally"
          onRemove={(i) => set({ dns: dns.filter((_, j) => j !== i) })}
          render={(d, i) => <DnsRow value={d} onChange={(next) => set({ dns: dns.map((x, j) => (j === i ? next : x)) })} />}
        />
        <AddButton onClick={() => set({ dns: [...dns, { enabled: true, host: "", ip: "127.0.0.1" }] })} testId="network-add-dns">
          Add DNS override
        </AddButton>
      </Section>
    </Page>
  );
}

/** A bordered list of editable rows, each with an enable switch on the left and a remove button. */
function Rows<T extends { enabled: boolean }>({
  items,
  empty,
  render,
  onRemove,
}: {
  items: T[];
  empty: string;
  render: (item: T, index: number) => ReactNode;
  onRemove: (index: number) => void;
}) {
  if (items.length === 0) return <div className="text-muted-foreground bg-muted/30 rounded-md border px-2 py-1.5 italic">{empty}</div>;
  return (
    <ul className="bg-muted/30 rounded-md border">
      {items.map((item, i) => (
        <li key={i} className="flex items-start gap-2 border-b px-2 py-1.5 last:border-b-0" data-testid="network-row">
          <div className="min-w-0 flex-1">{render(item, i)}</div>
          <Button variant="ghost" size="icon-xs" aria-label="Remove" onClick={() => onRemove(i)}>
            <Trash2 />
          </Button>
        </li>
      ))}
    </ul>
  );
}

function AddButton({ children, onClick, testId }: { children: ReactNode; onClick: () => void; testId: string }) {
  return (
    <div>
      <Button variant="outline" size="sm" onClick={onClick} data-testid={testId}>
        <Plus /> {children}
      </Button>
    </div>
  );
}

function Enabled({ checked, onChange }: { checked: boolean; onChange: (on: boolean) => void }) {
  return <Switch size="sm" className="mt-1.5" checked={checked} onCheckedChange={onChange} aria-label="Enabled" />;
}

/** A labelled control; `wide` grows to fill the line, `full` takes a line of its own. */
function Field({ label, children, wide, full }: { label: string; children: ReactNode; wide?: boolean; full?: boolean }) {
  return (
    <label
      className={cn(
        "flex flex-col gap-0.5",
        wide && "min-w-48 flex-1",
        full && "basis-full",
      )}
    >
      <span className="text-muted-foreground text-[11px]">{label}</span>
      {children}
    </label>
  );
}

const hostsText = (hosts: string[]) => hosts.join(", ");
const parseHosts = (text: string) => text.split(",").map((h) => h.trim());

function PortInput({ value, onChange, testId }: { value: number; onChange: (port: number) => void; testId?: string }) {
  return (
    <Input
      type="number"
      inputMode="numeric"
      min={1}
      max={65535}
      value={Number.isFinite(value) && value > 0 ? value : ""}
      onChange={(e) => onChange(Number(e.target.value))}
      className="w-20 font-mono tabular-nums"
      data-testid={testId}
    />
  );
}

/** A write-only password: empty means "keep the saved one". */
function PasswordInput({
  saved,
  value,
  onChange,
}: {
  saved: boolean;
  value: string | null | undefined;
  onChange: (password: string | null) => void;
}) {
  return (
    <Input
      type="password"
      autoComplete="off"
      value={value ?? ""}
      placeholder={saved ? "Saved in the keychain" : "None"}
      onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
      className="w-36"
    />
  );
}

function UpstreamRow({
  rule,
  onChange,
  index,
}: {
  rule: UpstreamRule_Deserialize;
  onChange: (rule: UpstreamRule_Deserialize) => void;
  index: number;
}) {
  const proxy = rule.proxy;
  const server: ProxyServer_Deserialize | null = proxy.kind === "direct" ? null : proxy;
  const setServer = (patch: Partial<ProxyServer_Deserialize>) =>
    proxy.kind !== "direct" && onChange({ ...rule, proxy: { ...proxy, ...patch } });
  const setKind = (kind: UpstreamProxy_Deserialize["kind"]) =>
    onChange({
      ...rule,
      proxy:
        kind === "direct"
          ? { kind }
          : { kind, ...(server ?? {}), host: server?.host ?? "", port: server?.port || (kind === "socks5" ? 1080 : 3128) },
    });
  return (
    <div className="flex items-start gap-2" data-testid={`network-upstream-${index}`}>
      <Enabled checked={rule.enabled} onChange={(enabled) => onChange({ ...rule, enabled })} />
      <div className="flex min-w-0 flex-1 flex-wrap items-end gap-2">
        <Field label="Hosts (comma separated)" full>
          <Input
            value={hostsText(rule.hosts)}
            onChange={(e) => onChange({ ...rule, hosts: parseHosts(e.target.value) })}
            placeholder="*.example.com, api.test"
            spellCheck={false}
            className="font-mono"
          />
        </Field>
        <Field label="Through">
          <Select value={proxy.kind} onValueChange={(k) => setKind(k as UpstreamProxy_Deserialize["kind"])}>
            <SelectTrigger size="sm" className="w-28">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="http">HTTP</SelectItem>
              <SelectItem value="socks5">SOCKS5</SelectItem>
              <SelectItem value="direct">Direct</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        {server && (
          <>
            <Field label="Proxy host">
              <Input
                value={server.host ?? ""}
                onChange={(e) => setServer({ host: e.target.value })}
                placeholder="proxy.corp.example"
                spellCheck={false}
                className="w-44 font-mono"
              />
            </Field>
            <Field label="Port">
              <PortInput value={server.port ?? 0} onChange={(port) => setServer({ port })} />
            </Field>
            <Field label="Username">
              <Input
                value={server.username ?? ""}
                onChange={(e) => setServer({ username: e.target.value || null })}
                placeholder="None"
                spellCheck={false}
                autoComplete="off"
                className="w-28"
              />
            </Field>
            <Field label="Password">
              <PasswordInput
                saved={server.has_password ?? false}
                value={server.password}
                onChange={(password) => setServer({ password })}
              />
            </Field>
          </>
        )}
      </div>
    </div>
  );
}

function ReverseRow({
  value,
  listener,
  failed,
  onChange,
}: {
  value: ReverseProxy;
  listener: ListenerStatus_Serialize | undefined;
  failed: ListenerStatus_Serialize | undefined;
  onChange: (value: ReverseProxy) => void;
}) {
  const scheme = value.tls ? "https" : "http";
  return (
    <div className="flex items-start gap-2">
      <Enabled checked={value.enabled} onChange={(enabled) => onChange({ ...value, enabled })} />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-end gap-2">
          <Field label="Local port">
            <PortInput value={value.listen_port} onChange={(listen_port) => onChange({ ...value, listen_port })} />
          </Field>
          <Field label="Forwards to" wide>
            <Input
              value={value.target_url}
              onChange={(e) => onChange({ ...value, target_url: e.target.value })}
              placeholder="https://api.example.com"
              spellCheck={false}
              className="font-mono"
            />
          </Field>
          <label className="flex h-7 items-center gap-1.5">
            <Switch size="sm" checked={value.tls} onCheckedChange={(tls) => onChange({ ...value, tls })} />
            HTTPS
          </label>
          <label className="flex h-7 items-center gap-1.5">
            <Switch
              size="sm"
              checked={value.preserve_host}
              onCheckedChange={(preserve_host) => onChange({ ...value, preserve_host })}
            />
            Keep Host
          </label>
        </div>
        {value.enabled && (
          <span className="text-muted-foreground font-mono text-[11px]">
            {listener?.addr
              ? `${scheme}://${listener.addr} → ${value.target_url}`
              : failed?.error
                ? <span className="text-[var(--s-failed)]">{failed.error}</span>
                : "Starts when saved"}
          </span>
        )}
      </div>
    </div>
  );
}

function CertRow({ value, onChange }: { value: ClientCert_Deserialize; onChange: (value: ClientCert_Deserialize) => void }) {
  const identity = value.identity;
  return (
    <div className="flex items-start gap-2">
      <Enabled checked={value.enabled} onChange={(enabled) => onChange({ ...value, enabled })} />
      <div className="flex min-w-0 flex-1 flex-wrap items-end gap-2">
        <Field label="Hosts (comma separated)">
          <Input
            value={hostsText(value.hosts)}
            onChange={(e) => onChange({ ...value, hosts: parseHosts(e.target.value) })}
            placeholder="mtls.example.com"
            spellCheck={false}
            className="w-44 font-mono"
          />
        </Field>
        <Field label="Format">
          <Select
            value={identity.kind}
            onValueChange={(kind) =>
              onChange({
                ...value,
                identity: kind === "pem" ? { kind: "pem", cert_path: "", key_path: "" } : { kind: "pkcs12", path: "", password: null },
              })
            }
          >
            <SelectTrigger size="sm" className="w-28">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="pkcs12">PKCS#12</SelectItem>
              <SelectItem value="pem">PEM</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        {identity.kind === "pkcs12" ? (
          <>
            <Field label=".p12 / .pfx file" wide>
              <Input
                value={identity.path}
                onChange={(e) => onChange({ ...value, identity: { ...identity, path: e.target.value } })}
                placeholder="/Users/me/certs/client.p12"
                spellCheck={false}
                className="font-mono"
              />
            </Field>
            <Field label="Password">
              <PasswordInput
                saved={identity.has_password ?? false}
                value={identity.password}
                onChange={(password) => onChange({ ...value, identity: { ...identity, password } })}
              />
            </Field>
          </>
        ) : (
          <>
            <Field label="Certificate (PEM)" wide>
              <Input
                value={identity.cert_path}
                onChange={(e) => onChange({ ...value, identity: { ...identity, cert_path: e.target.value } })}
                placeholder="/Users/me/certs/client.pem"
                spellCheck={false}
                className="font-mono"
              />
            </Field>
            <Field label="Key (PEM, optional)" wide>
              <Input
                value={identity.key_path}
                onChange={(e) => onChange({ ...value, identity: { ...identity, key_path: e.target.value } })}
                placeholder="In the certificate file"
                spellCheck={false}
                className="font-mono"
              />
            </Field>
          </>
        )}
      </div>
    </div>
  );
}

function DnsRow({ value, onChange }: { value: DnsOverride; onChange: (value: DnsOverride) => void }) {
  return (
    <div className="flex items-start gap-2">
      <Enabled checked={value.enabled} onChange={(enabled) => onChange({ ...value, enabled })} />
      <div className="flex min-w-0 flex-1 flex-wrap items-end gap-2">
        <Field label="Host" wide>
          <Input
            value={value.host}
            onChange={(e) => onChange({ ...value, host: e.target.value })}
            placeholder="api.example.com or *.staging.test"
            spellCheck={false}
            className="font-mono"
          />
        </Field>
        <Field label="Connect to">
          <Input
            value={value.ip}
            onChange={(e) => onChange({ ...value, ip: e.target.value })}
            placeholder="127.0.0.1"
            spellCheck={false}
            className="w-40 font-mono"
          />
        </Field>
      </div>
    </div>
  );
}
