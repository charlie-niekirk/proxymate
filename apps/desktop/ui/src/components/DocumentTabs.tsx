import { Archive, FileJson, Radio, X } from "lucide-react";

import { closeDocument } from "@/lib/actions";
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
      <Tab active={store === LIVE_STORE} onClick={() => setStore(LIVE_STORE)} testId="tab-live">
        <Radio className="size-3 text-[var(--s-2xx)]" />
        Live
      </Tab>
      {documents.map((doc) => (
        <Tab key={doc.store} active={store === doc.store} onClick={() => setStore(doc.store)} title={doc.path} testId={`tab-${doc.store}`}>
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
      ))}
    </div>
  );
}

function Tab({
  active,
  onClick,
  title,
  testId,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title?: string;
  testId?: string;
  children: React.ReactNode;
}) {
  return (
    <button
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
