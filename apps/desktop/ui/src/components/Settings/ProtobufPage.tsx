import { useQuery } from "@tanstack/react-query";
import { FileCode2, Trash2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { decodedRootKey, protoMappingsKey, protoSchemasKey, protoTypesKey, queryClient } from "@/lib/query";

import { Note, Page, Section } from "./parts";

/** Schemas and mappings changed: everything decoded with them is stale. */
async function refresh() {
  await Promise.all(
    [protoSchemasKey, protoTypesKey, protoMappingsKey, decodedRootKey].map((queryKey) => queryClient.invalidateQueries({ queryKey })),
  );
}

/**
 * Settings › Protobuf (M16): imported `.proto` files and descriptor sets,
 * which decode gRPC calls by their path, and the "Decode as…" mappings
 * for protobuf that isn't gRPC.
 */
export function ProtobufPage() {
  const schemas = useQuery({ queryKey: protoSchemasKey, queryFn: () => backend.listProtoSchemas() });
  const mappings = useQuery({ queryKey: protoMappingsKey, queryFn: () => backend.listProtoMappings() });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async (action: () => Promise<unknown>) => {
    setError(null);
    setBusy(true);
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Page testId="settings-protobuf">
      <Section
        title="Schemas"
        description={
          <>
            <code>.proto</code> files (compiled here, imports resolved from their folder) or a descriptor set from{" "}
            <code>buf build -o</code> or <code>protoc --descriptor_set_out</code>. gRPC calls find their message types from the
            path; without a schema, bodies decode schema-less.
          </>
        }
        aside={
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void run(() => backend.importProtoSchema(null))} data-testid="import-proto">
            Import…
          </Button>
        }
      >
        {error && <Note error={error} />}
        {schemas.data?.length ? (
          <ul className="divide-y rounded-md border" data-testid="proto-schemas">
            {schemas.data.map((s) => (
              <li key={s.id} className="flex items-start gap-2 px-2 py-1.5" data-testid="proto-schema">
                <FileCode2 className="text-muted-foreground mt-0.5 size-3.5 shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="font-medium">{s.name}</div>
                  <div className="text-muted-foreground truncate font-mono text-[11px]" title={s.files.join("\n")}>
                    {s.files.join(", ")}
                  </div>
                  <div className="text-muted-foreground text-[11px]">
                    {s.messages} message type{s.messages === 1 ? "" : "s"}
                    {s.services.length > 0 && ` · ${s.services.join(", ")}`}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove ${s.name}`}
                  disabled={busy}
                  onClick={() => void run(() => backend.removeProtoSchema(s.id))}
                >
                  <Trash2 />
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-muted-foreground" data-testid="no-proto-schemas">
            No schemas yet.
          </p>
        )}
      </Section>
      <Section
        title="Decode as"
        description="Protobuf bodies outside gRPC decode as the message type picked in the Body tab's “Decode as” menu, for that host and path."
      >
        {mappings.data?.length ? (
          <ul className="divide-y rounded-md border" data-testid="proto-mappings">
            {mappings.data.map((m) => (
              <li key={m.id} className="flex items-center gap-2 px-2 py-1">
                <span className="min-w-0 flex-1 truncate font-mono text-[11px]">
                  {m.matcher.host?.value ?? "*"}
                  {m.matcher.path?.value ?? ""} {m.part ? `(${m.part})` : ""}
                </span>
                <span className="font-mono text-[11px]">{m.message_type}</span>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label="Remove mapping"
                  disabled={busy}
                  onClick={() => void run(() => backend.setProtoMappings(mappings.data.filter((x) => x.id !== m.id)))}
                >
                  <Trash2 />
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-muted-foreground">None.</p>
        )}
      </Section>
    </Page>
  );
}
