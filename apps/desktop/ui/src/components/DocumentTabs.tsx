import { Archive, FileJson, Radio, X } from "lucide-react";
import type { ComponentProps, ReactElement } from "react";

import { CopyMenu } from "@/components/menus/items";
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu";
import { closeDocument, exportHar, saveSession } from "@/lib/actions";
import { formatCount } from "@/lib/flow";
import { cn } from "@/lib/utils";
import { LIVE_STORE, useUi } from "@/store/ui";

/**
 * Document tabs: the live capture, then each opened session or HAR. Shown
 * only while a document is open; the table and inspector follow the tab.
 */
export function DocumentTabs() {
  const documents = useUi((s) => s.documents);
  const store = useUi((s) => s.store);
  const setStore = useUi((s) => s.setStore);
  if (documents.length === 0) return null;
  return (
    <div className="bg-pane-toolbar flex h-7 shrink-0 items-end gap-0.5 overflow-x-auto border-b px-2" data-testid="document-tabs">
      <TabMenu store={LIVE_STORE}>
        <Tab active={store === LIVE_STORE} onClick={() => setStore(LIVE_STORE)} testId="tab-live">
          <Radio className="size-3 text-[var(--s-2xx)]" />
          Live
        </Tab>
      </TabMenu>
      {documents.map((doc) => (
        <TabMenu key={doc.store} store={doc.store} name={doc.name} path={doc.path}>
          <Tab active={store === doc.store} onClick={() => setStore(doc.store)} title={doc.path} testId={`tab-${doc.store}`}>
            {doc.kind === "har" ? <FileJson className="size-3" /> : <Archive className="size-3" />}
            <span className="max-w-48 truncate">{doc.name}</span>
            <span className="text-muted-foreground text-2xs tabular-nums">{formatCount(doc.flows)}</span>
            <span
              role="button"
              aria-label={`Close ${doc.name}`}
              data-testid={`close-${doc.store}`}
              className="hover:bg-accent -mr-1 rounded-sm p-0.5 opacity-60 hover:opacity-100"
              onClick={(e) => {
                e.stopPropagation();
                void closeDocument(doc.store);
              }}
            >
              <X className="size-3" />
            </span>
          </Tab>
        </TabMenu>
      ))}
    </div>
  );
}

/**
 * A tab's menu: copy its file's name or path, save or export it, close it.
 * Saving acts on the active tab, so it switches to this one first.
 */
function TabMenu({ store, name, path, children }: { store: number; name?: string; path?: string; children: ReactElement }) {
  const run = (save: () => Promise<void>) => {
    useUi.getState().setStore(store);
    void save();
  };
  return (
    <CopyMenu
      testId="tab-menu"
      items={[name && { label: "Copy name", text: name, what: "name" }, path && { label: "Copy path", text: path, what: "path" }]}
      extra={
        <>
          <ContextMenuItem onSelect={() => run(() => saveSession())}>Save session…</ContextMenuItem>
          <ContextMenuItem onSelect={() => run(() => exportHar({ kind: "all" }))}>Export HAR…</ContextMenuItem>
          {store !== LIVE_STORE && (
            <>
              <ContextMenuSeparator />
              <ContextMenuItem onSelect={() => void closeDocument(store)}>Close</ContextMenuItem>
            </>
          )}
        </>
      }
    >
      {children}
    </CopyMenu>
  );
}

function Tab({
  active,
  onClick,
  title,
  testId,
  children,
  ...rest
}: Omit<ComponentProps<"button">, "onClick"> & {
  active: boolean;
  onClick: () => void;
  title?: string;
  testId?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      {...rest}
      onClick={onClick}
      title={title}
      data-active={active}
      data-testid={testId}
      className={cn(
        "text-muted-foreground hover:text-foreground flex h-6 shrink-0 items-center gap-1.5 rounded-t-md border border-b-0 border-transparent px-2.5 text-xs",
        active && "bg-background text-foreground border-border",
      )}
    >
      {children}
    </button>
  );
}
