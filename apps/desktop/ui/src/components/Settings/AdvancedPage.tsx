import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { ProxySettings_Serialize, RuleSettings, ScriptSettings_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { proxySettingsKey, queryClient, ruleSettingsKey, scriptSettingsKey } from "@/lib/query";

import { Note, NumberField, Page, Section } from "./parts";

/** Settings › Advanced: how long breakpoints wait and scripts may run, and the upstream connection pool. */
export function AdvancedPage() {
  const { data: rules } = useQuery({ queryKey: ruleSettingsKey, queryFn: () => backend.getRuleSettings() });
  const { data: proxy } = useQuery({ queryKey: proxySettingsKey, queryFn: () => backend.getProxySettings() });
  const { data: scripts } = useQuery({ queryKey: scriptSettingsKey, queryFn: () => backend.getScriptSettings() });
  return (
    <Page testId="settings-advanced">
      {rules && <BreakpointForm key={JSON.stringify(rules)} settings={rules} />}
      {scripts && <ScriptTimeoutsForm key={JSON.stringify(scripts)} settings={scripts} />}
      {proxy && <PoolForm key={JSON.stringify(proxy)} settings={proxy} />}
    </Page>
  );
}

function BreakpointForm({ settings }: { settings: RuleSettings }) {
  const [timeout, setTimeoutSecs] = useState(settings.breakpoint_timeout_secs);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    setError(null);
    try {
      queryClient.setQueryData(ruleSettingsKey, await backend.setRuleSettings({ breakpoint_timeout_secs: timeout }));
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <Section
      title="Breakpoints"
      description="A paused flow waits this long for you, then continues unmodified (and is marked as timed out). 0 waits forever."
    >
      <form
        className="flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <NumberField label="Timeout" value={timeout} onChange={setTimeoutSecs} unit="seconds" testId="settings-bp-timeout" />
        <Button type="submit" variant="outline" size="sm" disabled={timeout === settings.breakpoint_timeout_secs || !Number.isFinite(timeout)}>
          Save
        </Button>
      </form>
      <Note error={error} />
    </Section>
  );
}

function ScriptTimeoutsForm({ settings }: { settings: ScriptSettings_Serialize }) {
  const [draft, setDraft] = useState(settings);
  const [error, setError] = useState<string | null>(null);
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  const save = async () => {
    setError(null);
    try {
      queryClient.setQueryData(scriptSettingsKey, await backend.setScriptSettings(draft));
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <Section
      title="Scripts"
      description="How long a script may run before it is stopped; the flow then goes on without it. 10 ms to 60 s."
    >
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <NumberField
          label="Rewrite and mock scripts"
          value={draft.traffic_timeout_ms}
          onChange={(traffic_timeout_ms) => setDraft({ ...draft, traffic_timeout_ms })}
          min={10}
          max={60_000}
          unit="ms"
          testId="settings-script-traffic-timeout"
        />
        <NumberField
          label="View scripts"
          value={draft.view_timeout_ms}
          onChange={(view_timeout_ms) => setDraft({ ...draft, view_timeout_ms })}
          min={10}
          max={60_000}
          unit="ms"
          testId="settings-script-view-timeout"
        />
        <div>
          <Button type="submit" size="sm" disabled={!changed || !Object.values(draft).every(Number.isFinite)}>
            Save
          </Button>
        </div>
        <Note error={error} />
      </form>
    </Section>
  );
}

function PoolForm({ settings }: { settings: ProxySettings_Serialize }) {
  const [draft, setDraft] = useState(settings);
  const [error, setError] = useState<string | null>(null);
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  const save = async () => {
    setError(null);
    try {
      queryClient.setQueryData(proxySettingsKey, await backend.setProxySettings(draft));
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <Section
      title="Upstream connections"
      description="Plain HTTP requests share a pool of connections to each server. Decrypted HTTPS opens one upstream connection per client connection. Saving restarts the listener."
    >
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <NumberField
          label="Keep idle connections for"
          value={draft.pool_idle_secs}
          onChange={(pool_idle_secs) => setDraft({ ...draft, pool_idle_secs })}
          unit="seconds"
          testId="settings-pool-idle"
        />
        <NumberField
          label="Idle connections per host"
          value={draft.pool_max_idle_per_host}
          onChange={(pool_max_idle_per_host) => setDraft({ ...draft, pool_max_idle_per_host })}
          max={1024}
          hint="0 closes every connection after its request."
          testId="settings-pool-max"
        />
        <NumberField
          label="Connect timeout"
          value={draft.connect_timeout_secs}
          onChange={(connect_timeout_secs) => setDraft({ ...draft, connect_timeout_secs })}
          min={1}
          max={300}
          unit="seconds"
          testId="settings-connect-timeout"
        />
        <div>
          <Button type="submit" size="sm" disabled={!changed || !Object.values(draft).every(Number.isFinite)}>
            Save
          </Button>
        </div>
        <Note error={error} />
      </form>
    </Section>
  );
}
