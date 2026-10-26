import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { StorageSettings_Serialize } from "@/bindings";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { clearFlows, errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatSize } from "@/lib/flow";
import { queryClient, storageSettingsKey } from "@/lib/query";
import { useStorage } from "@/store/storage";

import { Note, NumberField, Page, Section } from "./parts";

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
    </Page>
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
