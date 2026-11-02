import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { AutoSaveSettings_Serialize, RecordingSettings_Serialize, StorageSettings_Serialize } from "@/bindings";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { autosaveNow, clearFlows, errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatSize } from "@/lib/flow";
import { autosaveSettingsKey, queryClient, recordingSettingsKey, storageSettingsKey } from "@/lib/query";
import { useStorage } from "@/store/storage";

import { ListEditor, Note, NumberField, Page, Section } from "./parts";

const nf = new Intl.NumberFormat();

/**
 * Settings › Storage: what the live capture holds and the limits that
 * evict its oldest flows (pinned and in-flight ones are kept).
 */
export function StoragePage() {
  const status = useStorage((s) => s.status);
  const { data: settings } = useQuery({ queryKey: storageSettingsKey, queryFn: () => backend.getStorageSettings() });
  return (
    <Page testId="settings-storage">
      <Section title="Live capture" description="Bodies and details are kept in files that are deleted on Clear and on quit.">
        {status && (
          <CopyMenu items={[{ label: "Copy location", text: status.dir, what: "path" }]}>
            <dl className="grid grid-cols-[180px_1fr] gap-x-3 gap-y-1" data-selectable data-testid="storage-status">
              <dt className="text-muted-foreground">Flows</dt>
              <dd className="tabular-nums">
                {nf.format(status.flows)}
                {status.pinned > 0 && ` (${nf.format(status.pinned)} pinned)`}
              </dd>
              <dt className="text-muted-foreground">On disk</dt>
              <dd className="tabular-nums">{status.disk_bytes > 0 ? formatSize(status.disk_bytes) : "Nothing yet"}</dd>
              <dt className="text-muted-foreground">Evicted since launch</dt>
              <dd className="tabular-nums">{nf.format(status.evicted)} flows</dd>
              <dt className="text-muted-foreground">Not recorded since launch</dt>
              <dd className="tabular-nums" data-testid="storage-unrecorded">
                {nf.format(status.unrecorded)} {status.unrecorded === 1 ? "flow" : "flows"}
              </dd>
              <dt className="text-muted-foreground">Location</dt>
              <dd className="truncate font-mono text-[11px]" title={status.dir}>
                {status.dir}
              </dd>
            </dl>
          </CopyMenu>
        )}
        {status?.error && <Note error={`Bodies aren't being recorded: ${status.error}.`} />}
        <div>
          <Button variant="outline" size="sm" onClick={() => void clearFlows()}>
            Clear all flows
          </Button>
        </div>
      </Section>
      {settings && <Limits key={JSON.stringify(settings)} settings={settings} />}
      <RecordingLists />
      <AutoSave />
    </Page>
  );
}

/** Hosts left out of the capture (M21): still proxied, never recorded. */
function RecordingLists() {
  const { data: lists } = useQuery({ queryKey: recordingSettingsKey, queryFn: () => backend.getRecordingSettings() });
  const [error, setError] = useState<string | null>(null);
  if (!lists) return null;
  const save = async (next: RecordingSettings_Serialize) => {
    setError(null);
    try {
      queryClient.setQueryData(recordingSettingsKey, await backend.setRecordingSettings(next));
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <>
      <Section
        title="Never record"
        description="Traffic to these hosts is still proxied, decrypted and rewritten by rules, but leaves no flow. *.example.com also covers example.com. Right-click a flow for “Never record this host”."
      >
        <ListEditor
          items={lists.never}
          onChange={(never) => void save({ ...lists, never })}
          placeholder="*.apple.com"
          empty="Every host is recorded"
          testId="recording-never"
        />
      </Section>
      <Section
        title="Only record"
        description="When this list has hosts, flows to any other host aren't recorded. Never record wins over it."
      >
        <ListEditor
          items={lists.only}
          onChange={(only) => void save({ ...lists, only })}
          placeholder="api.example.com"
          empty="No allow-list: every host not above is recorded"
          testId="recording-only"
        />
        <Note error={error} />
      </Section>
    </>
  );
}

const timeFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/** Saves the live capture every N minutes as a rotating `.proxymate` (M21). */
function AutoSave() {
  const { data: settings } = useQuery({ queryKey: autosaveSettingsKey, queryFn: () => backend.getAutosaveSettings() });
  if (!settings) return null;
  return <AutoSaveForm key={JSON.stringify(settings)} settings={settings} />;
}

function AutoSaveForm({ settings }: { settings: AutoSaveSettings_Serialize }) {
  const status = useStorage((s) => s.status?.autosave ?? null);
  const [draft, setDraft] = useState(settings);
  const [error, setError] = useState<string | null>(null);
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  const valid = Number.isFinite(draft.interval_minutes) && Number.isFinite(draft.keep);
  const save = async (next: AutoSaveSettings_Serialize) => {
    setError(null);
    try {
      queryClient.setQueryData(autosaveSettingsKey, await backend.setAutosaveSettings(next));
    } catch (e) {
      setError(errorText(e));
    }
  };
  const choose = async () => {
    try {
      const folder = await backend.chooseAutosaveFolder();
      if (folder) setDraft({ ...draft, folder });
    } catch (e) {
      setError(errorText(e));
    }
  };
  const name = status?.last_path?.split("/").pop();
  return (
    <Section
      title="Auto-save"
      description="Saves the live capture as a session in a folder, keeping the newest few. Nothing is written while the capture hasn't changed."
      aside={
        <Switch
          checked={draft.enabled}
          onCheckedChange={(enabled) => void save({ ...draft, enabled })}
          aria-label="Auto-save the live capture"
          data-testid="autosave-enabled"
        />
      }
    >
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void save(draft);
        }}
      >
        <label className="grid grid-cols-[180px_1fr] items-center gap-x-3">
          <span>Folder</span>
          <span className="flex min-w-0 items-center gap-2">
            <Input
              value={draft.folder}
              onChange={(e) => setDraft({ ...draft, folder: e.target.value })}
              placeholder="/Users/you/Documents/Proxymate"
              spellCheck={false}
              className="min-w-0 flex-1 font-mono"
              data-testid="autosave-folder"
            />
            <Button type="button" variant="outline" size="sm" onClick={() => void choose()}>
              Choose…
            </Button>
          </span>
        </label>
        <NumberField
          label="Every"
          value={draft.interval_minutes}
          onChange={(interval_minutes) => setDraft({ ...draft, interval_minutes })}
          unit="minutes"
          min={1}
          max={1440}
          testId="autosave-interval"
        />
        <NumberField
          label="Keep"
          value={draft.keep}
          onChange={(keep) => setDraft({ ...draft, keep })}
          unit="sessions"
          hint="Older Proxymate Autosave files in the folder are deleted; nothing else there is touched."
          min={1}
          max={1000}
          testId="autosave-keep"
        />
        <div className="flex items-center gap-2">
          <Button type="submit" size="sm" disabled={!changed || !valid} data-testid="autosave-save">
            Save
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={!settings.folder || status?.saving}
            onClick={() => void autosaveNow()}
            data-testid="autosave-now"
          >
            Save now
          </Button>
          <span className="text-muted-foreground truncate" data-testid="autosave-status" title={status?.last_path ?? undefined}>
            {status?.saving
              ? "Saving…"
              : status?.last_at_ms
                ? `Last saved ${timeFormat.format(status.last_at_ms)} · ${name}`
                : settings.enabled
                  ? "Not saved yet"
                  : ""}
          </span>
        </div>
        <Note error={error ?? (status?.error ? `The last auto-save failed: ${status.error}` : null)} />
      </form>
    </Section>
  );
}

function Limits({ settings }: { settings: StorageSettings_Serialize }) {
  const [draft, setDraft] = useState(settings);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  const valid = Object.values(draft).every(Number.isFinite);
  const save = async () => {
    setError(null);
    setSaved(null);
    try {
      queryClient.setQueryData(storageSettingsKey, await backend.setStorageSettings(draft));
      setSaved("Saved. The capture was trimmed to the new limits.");
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <Section
      title="Limits"
      description="Past a limit the oldest flows are evicted, a twentieth at a time, and their files deleted. Pinned flows and flows still in flight are kept. 0 means no limit."
    >
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <NumberField
          label="Flows kept"
          value={draft.max_flows}
          onChange={(max_flows) => setDraft({ ...draft, max_flows })}
          unit="flows"
          hint="250,000 by default. About 200 bytes of memory each, plus their bodies on disk."
          testId="storage-max-flows"
        />
        <NumberField
          label="Disk used"
          value={draft.max_disk_mb}
          onChange={(max_disk_mb) => setDraft({ ...draft, max_disk_mb })}
          unit="MB"
          hint="Bodies over 8 KB and every flow's details live in 64 MB files; whole files are freed."
          testId="storage-max-disk"
        />
        <NumberField
          label="WebSocket messages"
          value={draft.max_ws_messages}
          onChange={(max_ws_messages) => setDraft({ ...draft, max_ws_messages })}
          unit="per connection"
          hint="A long-lived connection keeps its newest messages; older ones are dropped."
          testId="storage-max-ws"
        />
        <div className="flex items-center gap-2">
          <Button type="submit" size="sm" disabled={!changed || !valid} data-testid="storage-save">
            Save limits
          </Button>
          {changed && (
            <Button type="button" variant="ghost" size="sm" onClick={() => setDraft(settings)}>
              Revert
            </Button>
          )}
        </div>
        <Note error={error} saved={saved} />
      </form>
    </Section>
  );
}
