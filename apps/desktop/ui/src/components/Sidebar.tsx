import { useQuery } from "@tanstack/react-query";
import {
  Archive,
  ChevronDown,
  ChevronRight,
  FileJson,
  FolderOpen,
  Globe,
  Layers,
  Pin,
  Radio,
  Search,
  ShieldOff,
  Star,
  type LucideIcon,
} from "lucide-react";
import { memo, useEffect, useState, type ReactNode } from "react";

import type { AppCount_Deserialize, DomainNode } from "@/bindings";
import { AppIcon } from "@/components/AppIcon";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { addSslBypass, closeDocument, openDocument, setAppSslExcluded } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { copy } from "@/lib/clipboard";
import { formatCount } from "@/lib/flow";
import { sslSettingsKey } from "@/lib/query";
import { cn } from "@/lib/utils";
import { startSidebar, useSidebar } from "@/store/sidebar";
import { useStorage } from "@/store/storage";
import { LIVE_STORE, useUi } from "@/store/ui";

function Section({ title, children, defaultOpen = true }: { title: string; children: ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="mb-2">
      <button
        onClick={() => setOpen(!open)}
        className="text-muted-foreground hover:text-foreground flex w-full items-center gap-1 px-2 py-1 text-2xs font-semibold tracking-wide uppercase"
      >
        {title}
        <ChevronDown className={cn("size-3 transition-transform", !open && "-rotate-90")} />
      </button>
      {open && children}
    </div>
  );
}

const itemClass = (active?: boolean) =>
  cn(
    "text-sidebar-foreground hover:bg-sidebar-accent flex h-6 w-full items-center gap-2 rounded-md px-2 text-left text-sm",
    active && "bg-sidebar-accent text-sidebar-accent-foreground font-medium",
  );

function Item({
  icon,
  label,
  count,
  active,
  muted,
  indent,
  onClick,
  testId,
}: {
  icon: ReactNode;
  label: string;
  count?: number;
  active?: boolean;
  muted?: boolean;
  indent?: boolean;
  onClick?: () => void;
  testId?: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(itemClass(active), muted && "text-muted-foreground italic", indent && "pl-7")}
      // Long app and host names are cut with an ellipsis; the whole name is here.
      title={count !== undefined ? `${label} · ${count.toLocaleString()} flows` : label}
      data-testid={testId}
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {count !== undefined && <span className="text-muted-foreground shrink-0 text-2xs tabular-nums">{formatCount(count)}</span>}
    </button>
  );
}

const glyph = (Icon: LucideIcon) => <Icon className="size-3.5 shrink-0 opacity-70" />;

/** The Apps and Domains filter box, for ⌘L. */
export const SIDEBAR_FILTER_ID = "sidebar-filter";

/** Pinned flows (kept when old flows are evicted): a click shows them. */
function PinnedItem() {
  const pinned = useStorage((s) => s.status?.pinned ?? 0);
  const live = useUi((s) => s.store === LIVE_STORE);
  const text = useUi((s) => s.filter.text);
  const setText = useUi((s) => s.setText);
  const showing = text.trim() === "is:pinned";
  if (pinned === 0 || !live) return <Item icon={glyph(Pin)} label="No pinned flows" muted />;
  return (
    <Item
      icon={glyph(Pin)}
      label="Pinned flows"
      count={pinned}
      active={showing}
      onClick={() => setText(showing ? "" : "is:pinned")}
      testId="sidebar-pinned"
    />
  );
}

/**
 * Opened sessions and HARs, beside the live capture: a click shows one in
 * the table (like its tab), the context menu closes it.
 */
function SavedDocuments() {
  const documents = useUi((s) => s.documents);
  const store = useUi((s) => s.store);
  const setStore = useUi((s) => s.setStore);
  return (
    <div data-testid="saved-documents">
      <Item
        icon={glyph(Radio)}
        label="Live capture"
        active={store === LIVE_STORE}
        onClick={() => setStore(LIVE_STORE)}
        testId="saved-live"
      />
      {documents.map((doc) => (
        <ContextMenu key={doc.store}>
          <ContextMenuTrigger asChild>
            <div title={doc.path}>
              <Item
                icon={glyph(doc.kind === "har" ? FileJson : Archive)}
                label={doc.name}
                count={doc.flows}
                active={store === doc.store}
                onClick={() => setStore(doc.store)}
                testId={`saved-${doc.store}`}
              />
            </div>
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem onSelect={() => void closeDocument(doc.store)}>Close {doc.name}</ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
      ))}
      <Item icon={glyph(FolderOpen)} label="Open session or HAR…" muted onClick={() => void openDocument()} />
    </div>
  );
}

/**
 * Sidebar over macOS vibrancy (solid colour elsewhere). Apps and Domains
 * come live from the engine, which aggregates their counts in Rust.
 */
export function Sidebar() {
  const host = useUi((s) => s.filter.host);
  const client = useUi((s) => s.filter.client);
  const setScope = useUi((s) => s.setScope);
  const snapshot = useSidebar((s) => s.snapshot);
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const match = (s: string) => s.toLowerCase().includes(needle);
  useEffect(startSidebar, []);

  const apps = snapshot.apps.filter((a) => match(a.name));
  const domains = snapshot.domains.filter((d) => match(d.domain) || d.hosts.some((h) => match(h.host)));

  return (
    <aside className="bg-sidebar flex h-full flex-col">
      {/* Traffic lights sit here; the strip drags the window. */}
      <div data-tauri-drag-region className="h-[52px] shrink-0" />
      {/* Radix wraps the content in a `display: table` box that grows with long names; a block lets them truncate. */}
      <ScrollArea className="min-h-0 flex-1 px-2 [&_[data-slot=scroll-area-viewport]>div]:!block">
        <Section title="Favorites">
          <Item icon={glyph(Star)} label="No favorites yet" muted />
        </Section>
        <Section title="Pinned">
          <PinnedItem />
        </Section>
        <Section title="Saved">
          <SavedDocuments />
        </Section>
        <Section title="All">
          <Item
            icon={glyph(Layers)}
            label="All traffic"
            count={snapshot.total}
            active={!host && !client}
            onClick={() => setScope({})}
          />
        </Section>
        <Section title="Apps">
          {apps.length === 0 && <Item icon={glyph(Layers)} label={needle ? "No matching apps" : "No apps yet"} muted />}
          {apps.map((app) => (
            <AppItem
              key={app.name}
              app={app}
              active={client === app.name}
              onSelect={() => setScope({ client: client === app.name ? null : app.name })}
            />
          ))}
        </Section>
        <Section title="Domains">
          {domains.length === 0 && <Item icon={glyph(Globe)} label={needle ? "No matching domains" : "No domains yet"} muted />}
          {domains.map((node) => (
            <DomainItem
              key={node.domain}
              node={node}
              selected={host}
              forceOpen={needle !== ""}
              onSelect={(h) => setScope({ host: host === h ? null : h })}
            />
          ))}
        </Section>
      </ScrollArea>
      <div className="relative shrink-0 p-2">
        <Search className="text-muted-foreground absolute top-1/2 left-4 size-3 -translate-y-1/2" />
        <Input
          id={SIDEBAR_FILTER_ID}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setQuery("")}
          placeholder="Filter apps and domains"
          aria-label="Filter apps and domains"
          className="bg-background/40 pl-6"
        />
      </div>
    </aside>
  );
}

const AppItem = memo(function AppItem({
  app,
  active,
  onSelect,
}: {
  app: AppCount_Deserialize;
  active: boolean;
  onSelect: () => void;
}) {
  const { data: ssl } = useQuery({ queryKey: sslSettingsKey, queryFn: () => backend.getSslSettings() });
  // Excluded by bundle id when there is one, else by name.
  const key = app.bundle_id ?? app.name;
  const excluded = (ssl?.exclude_apps ?? []).some((a) => a.toLowerCase() === key.toLowerCase());
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div>
          <Item
            icon={
              <span className="relative shrink-0">
                <AppIcon name={app.name} />
                {excluded && <ShieldOff className="text-muted-foreground absolute -right-1 -bottom-1 size-2.5" />}
              </span>
            }
            label={app.name}
            count={app.count}
            active={active}
            onClick={onSelect}
            testId="sidebar-app"
          />
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent className="w-60">
        <ContextMenuItem onSelect={onSelect}>{active ? "Show all traffic" : `Show only ${app.name}`}</ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuCheckboxItem checked={excluded} onCheckedChange={(on) => void setAppSslExcluded(key, on === true)}>
          Don&apos;t decrypt HTTPS from {app.name}
        </ContextMenuCheckboxItem>
      </ContextMenuContent>
    </ContextMenu>
  );
});

const DomainItem = memo(function DomainItem({
  node,
  selected,
  forceOpen,
  onSelect,
}: {
  node: DomainNode;
  selected: string | null | undefined;
  forceOpen: boolean;
  onSelect: (host: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const expandable = node.hosts.length > 0;
  const expanded = expandable && (open || forceOpen);
  // A domain row filters to the domain and every host under it.
  const scope = expandable ? `*.${node.domain}` : node.domain;
  return (
    <>
      <HostMenu host={node.domain}>
        <div className={itemClass(selected === scope)}>
          <button
            aria-label={expanded ? "Collapse" : "Expand"}
            onClick={() => setOpen(!open)}
            className={cn("-ml-1 shrink-0 rounded-sm p-0.5", !expandable && "invisible")}
          >
            <ChevronRight className={cn("size-3 opacity-60 transition-transform", expanded && "rotate-90")} />
          </button>
          <button className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => onSelect(scope)} data-testid="sidebar-domain">
            {glyph(Globe)}
            <span className="min-w-0 flex-1 truncate">{node.domain}</span>
            <span className="text-muted-foreground shrink-0 text-2xs tabular-nums">{formatCount(node.count)}</span>
          </button>
        </div>
      </HostMenu>
      {expanded &&
        node.hosts.map((h) => (
          <HostMenu key={h.host} host={h.host}>
            <div>
              <Item
                icon={null}
                label={h.host}
                count={h.count}
                indent
                active={selected === h.host}
                onClick={() => onSelect(h.host)}
              />
            </div>
          </HostMenu>
        ))}
    </>
  );
});

function HostMenu({ host, children }: { host: string; children: ReactNode }) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-56">
        <ContextMenuItem onSelect={() => void copy(host, "host")}>Copy {host}</ContextMenuItem>
        <ContextMenuItem onSelect={() => void addSslBypass(host)}>Add {host} to SSL bypass</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

