import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { UpdateSettings } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { backend } from "@/lib/backend";
import { appVersionKey, queryClient, updateSettingsKey } from "@/lib/query";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * Settings › Updates: the quiet check at launch, and "Check Now" (the app
 * menu's "Check for Updates…", answered with a native dialog).
 */
export function UpdatesPage() {
  const { data: version } = useQuery({ queryKey: appVersionKey, queryFn: () => backend.appVersion() });
  const { data: settings } = useQuery({ queryKey: updateSettingsKey, queryFn: () => backend.getUpdateSettings() });
  const [error, setError] = useState<string | null>(null);
  const save = async (next: UpdateSettings) => {
    setError(null);
    try {
      queryClient.setQueryData(updateSettingsKey, await backend.setUpdateSettings(next));
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <div className="flex flex-col gap-5 p-4 text-xs" data-testid="settings-updates">
      <section className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="text-[13px] font-semibold">Proxymate {version ?? ""}</h3>
          <p className="text-muted-foreground mt-0.5 leading-4">
            Updates come from Proxymate's GitHub releases and are verified against its signing key before they're
            installed. Proxymate asks before installing and restarts afterwards.
          </p>
        </div>
        <Button size="sm" variant="outline" onClick={() => void backend.checkForUpdates()} data-testid="update-check">
          Check Now
        </Button>
      </section>
      <section className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="text-[13px] font-semibold">Check for updates at launch</h3>
          <p className="text-muted-foreground mt-0.5 leading-4">
            A quiet check a few seconds after Proxymate starts. You only hear about it when an update is available.
          </p>
        </div>
        <Switch
          checked={settings?.check_at_launch ?? true}
          disabled={!settings}
          onCheckedChange={(check_at_launch) => void save({ check_at_launch })}
          aria-label="Check for updates at launch"
          data-testid="update-check-at-launch"
        />
      </section>
      {error && <p className="text-[var(--s-5xx)]">{error}</p>}
    </div>
  );
}
