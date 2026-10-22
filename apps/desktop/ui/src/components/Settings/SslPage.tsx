import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { SslSettings } from "@/bindings";
import { errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { queryClient, sslSettingsKey } from "@/lib/query";
import { useSidebar } from "@/store/sidebar";

import { ListEditor, Note, Page, Section } from "./parts";

type ListKey = "include" | "exclude" | "bypass" | "insecure_upstream" | "exclude_apps";

const LISTS: { key: ListKey; title: string; description: string; placeholder: string; empty: string }[] = [
  {
    key: "include",
    title: "Decrypt",
    description: "Hosts whose HTTPS is decrypted. * decrypts everything not excluded below.",
    placeholder: "*.example.com",
    empty: "Nothing is decrypted: every HTTPS connection is tunnelled",
  },
  {
    key: "exclude",
    title: "Never decrypt",
    description: "Hosts that are tunnelled as is. Apple, iCloud and certificate checks are here by default: they pin, and breaking them breaks the Mac.",
    placeholder: "*.apple.com",
    empty: "No exclusions",
  },
  {
    key: "bypass",
    title: "SSL bypass",
    description: "Added with “Add to SSL bypass”, usually for apps that pin their certificates and refuse Proxymate's.",
    placeholder: "pinned.example.com",
    empty: "Nothing bypassed",
  },
  {
    key: "insecure_upstream",
    title: "Allow insecure upstream",
    description: "Hosts whose own certificate is accepted without verification, e.g. a dev server with a self-signed certificate. Signatures are still checked.",
    placeholder: "localhost",
    empty: "Every upstream certificate is verified",
  },
];

/**
 * Settings › SSL Proxying: which hosts and apps are decrypted. Patterns are
 * host globs (`*` crosses dots), compared ignoring case. New connections
 * use the lists straight away.
 */
export function SslPage() {
  const { data: settings } = useQuery({ queryKey: sslSettingsKey, queryFn: () => backend.getSslSettings() });
  const apps = useSidebar((s) => s.snapshot.apps);
  const [error, setError] = useState<string | null>(null);
  if (!settings) return null;
  const save = async (next: SslSettings) => {
    setError(null);
    try {
      queryClient.setQueryData(sslSettingsKey, await backend.setSslSettings(next));
    } catch (e) {
      setError(errorText(e));
    }
  };
  const knownApps = apps.map((a) => a.bundle_id ?? a.name).filter((a) => !(settings.exclude_apps ?? []).includes(a));
  return (
    <Page testId="settings-ssl">
      <Note error={error} />
      {LISTS.map((list) => (
        <Section key={list.key} title={list.title} description={list.description}>
          <ListEditor
            items={settings[list.key] ?? []}
            onChange={(items) => void save({ ...settings, [list.key]: items })}
            placeholder={list.placeholder}
            empty={list.empty}
            testId={`ssl-${list.key}`}
          />
        </Section>
      ))}
      <Section
        title="Apps never decrypted"
        description="Everything these apps send is tunnelled, whatever the host. Use a bundle id (com.example.App) or, for command-line tools, their name. Right-click an app in the sidebar to add it."
      >
        <ListEditor
          items={settings.exclude_apps ?? []}
          onChange={(items) => void save({ ...settings, exclude_apps: items })}
          placeholder={knownApps[0] ?? "com.example.App"}
          empty="Every app is decrypted"
          testId="ssl-exclude_apps"
        />
      </Section>
    </Page>
  );
}
