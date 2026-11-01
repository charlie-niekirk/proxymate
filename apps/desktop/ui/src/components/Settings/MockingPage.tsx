import { useQuery } from "@tanstack/react-query";
import { FileBox, FileCode2, Trash2, Wand2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { useMockSources } from "@/hooks/useMocking";
import { errorText, generateOpenApiMocks, importOpenApiSpec, mockFromFile, removeOpenApiSpec } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatSize } from "@/lib/flow";
import { mockSourcesKey, openApiSpecsKey, queryClient } from "@/lib/query";
import { useUi } from "@/store/ui";

import { Note, Page, Section } from "./parts";

/**
 * Settings › Mocking (M20): the mock library that "Serve from recording"
 * rules answer from, and imported OpenAPI specs, which label flows with
 * their operation and can generate mock rules.
 */
export function MockingPage() {
  const sources = useMockSources();
  const specs = useQuery({ queryKey: openApiSpecsKey, queryFn: () => backend.listOpenApiSpecs() });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async (action: () => Promise<unknown>) => {
    setError(null);
    setBusy(true);
    try {
      await action();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const closeSettings = useUi((s) => s.closeSettings);
  return (
    <Page testId="settings-mocking">
      <Section
        title="Recordings"
        description={
          <>
            Sessions copied in with “Mock from session…” (a tab’s menu) or from a file: a <code>.proxymate</code> session, a HAR,
            or a Charles export (<code>.chlsj</code>, <code>.chlsx</code>). A “Serve from recording” rule answers matching requests
            with what was recorded, without contacting the server.
          </>
        }
        aside={
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await mockFromFile();
                closeSettings();
              })
            }
            data-testid="mock-from-file"
          >
            Add from file…
          </Button>
        }
      >
        {error && <Note error={error} />}
        {sources.length ? (
          <ul className="divide-y rounded-md border" data-testid="mock-sources">
            {sources.map((s) => (
              <li key={s.id} className="flex items-start gap-2 px-2 py-1.5" data-testid="mock-source">
                <FileBox className="text-muted-foreground mt-0.5 size-3.5 shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="font-medium">{s.name}</div>
                  <div className="text-muted-foreground truncate font-mono text-[11px]" title={s.origin}>
                    {s.origin}
                  </div>
                  <div className="text-muted-foreground truncate text-[11px]" title={s.hosts.join("\n")}>
                    {s.flows} flow{s.flows === 1 ? "" : "s"} · {formatSize(s.size)}
                    {s.hosts.length > 0 && ` · ${s.hosts.join(", ")}`}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove ${s.name}`}
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await backend.removeMockSource(s.id);
                      await queryClient.invalidateQueries({ queryKey: mockSourcesKey });
                    })
                  }
                >
                  <Trash2 />
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-muted-foreground" data-testid="no-mock-sources">
            No recordings yet.
          </p>
        )}
      </Section>
      <Section
        title="OpenAPI specs"
        description={
          <>
            OpenAPI 3.0 and 3.1, JSON or YAML. Flows whose method and path match an operation show its <code>operationId</code> in the
            inspector, and <code>op:</code> in the search box finds them. “Generate mocks” adds one Map Local rule per operation, from
            its example or a minimal value of its response schema, turned off until you review it.
          </>
        }
        aside={
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void run(() => importOpenApiSpec())} data-testid="import-openapi">
            Import…
          </Button>
        }
      >
        {specs.data?.length ? (
          <ul className="divide-y rounded-md border" data-testid="openapi-specs">
            {specs.data.map((s) => (
              <li key={s.id} className="flex items-start gap-2 px-2 py-1.5" data-testid="openapi-spec">
                <FileCode2 className="text-muted-foreground mt-0.5 size-3.5 shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="font-medium">
                    {s.name} {s.version && <span className="text-muted-foreground font-normal">{s.version}</span>}
                  </div>
                  <div className="text-muted-foreground truncate text-[11px]" title={s.servers.join("\n")}>
                    OpenAPI {s.openapi} · {s.operations} operation{s.operations === 1 ? "" : "s"}
                    {s.servers.length > 0 && ` · ${s.servers.join(", ")}`}
                  </div>
                </div>
                <Button
                  variant="outline"
                  size="xs"
                  disabled={busy || s.operations === 0}
                  onClick={() =>
                    void run(async () => {
                      await generateOpenApiMocks(s.id);
                      closeSettings();
                    })
                  }
                  data-testid="generate-mocks"
                >
                  <Wand2 />
                  Generate mocks
                </Button>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove ${s.name}`}
                  disabled={busy}
                  onClick={() => void run(() => removeOpenApiSpec(s.id))}
                >
                  <Trash2 />
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-muted-foreground" data-testid="no-openapi-specs">
            No specs yet.
          </p>
        )}
      </Section>
    </Page>
  );
}
