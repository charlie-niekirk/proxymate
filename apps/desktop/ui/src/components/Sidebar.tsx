import { AppWindow, Archive, ChevronDown, Globe, Layers, Pin, Search, Star, type LucideIcon } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { useUi } from "@/store/ui";

// Static until the engine aggregates Apps/Domains counts (M4).
const APPS = ["Safari", "Google Chrome", "Firefox", "Slack", "Postman", "Spotify", "NetNewsWire", "node"];
const DOMAINS = [
  "api.github.com",
  "github.githubassets.com",
  "www.google.com",
  "fonts.gstatic.com",
  "api.stripe.com",
  "slack.com",
  "wss-primary.slack.com",
  "i.ytimg.com",
  "rr3---sn-a5mekn6s.googlevideo.com",
  "localhost",
  "feeds.example.org",
  "cdn.jsdelivr.net",
  "docs.example.com",
  "telemetry.example.net",
];

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

function Item({
  icon: Icon,
  label,
  active,
  muted,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  active?: boolean;
  muted?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "text-sidebar-foreground hover:bg-sidebar-accent flex h-6 w-full items-center gap-2 rounded-md px-2 text-left text-sm",
        active && "bg-sidebar-accent text-sidebar-accent-foreground font-medium",
        muted && "text-muted-foreground italic",
      )}
    >
      <Icon className="size-3.5 shrink-0 opacity-70" />
      <span className="truncate">{label}</span>
    </button>
  );
}

/** Sidebar over macOS vibrancy (solid colour elsewhere). */
export function Sidebar() {
  const host = useUi((s) => s.filter.host);
  const client = useUi((s) => s.filter.client);
  const setScope = useUi((s) => s.setScope);
  const [query, setQuery] = useState("");
  const match = (s: string) => s.toLowerCase().includes(query.trim().toLowerCase());

  return (
    <aside className="bg-sidebar flex h-full flex-col">
      {/* Traffic lights sit here; the strip drags the window. */}
      <div data-tauri-drag-region className="h-[52px] shrink-0" />
      <ScrollArea className="min-h-0 flex-1 px-2">
        <Section title="Favorites">
          <Item icon={Star} label="No favorites yet" muted />
        </Section>
        <Section title="Pinned">
          <Item icon={Pin} label="No pinned flows" muted />
        </Section>
        <Section title="Saved Sessions" defaultOpen={false}>
          <Item icon={Archive} label="Sessions arrive in M8" muted />
        </Section>
        <Section title="All">
          <Item icon={Layers} label="All traffic" active={!host && !client} onClick={() => setScope({})} />
        </Section>
        <Section title="Apps">
          {APPS.filter(match).map((app) => (
            <Item
              key={app}
              icon={AppWindow}
              label={app}
              active={client === app}
              onClick={() => setScope({ client: client === app ? null : app })}
            />
          ))}
        </Section>
        <Section title="Domains">
          {DOMAINS.filter(match).map((d) => (
            <Item key={d} icon={Globe} label={d} active={host === d} onClick={() => setScope({ host: host === d ? null : d })} />
          ))}
        </Section>
      </ScrollArea>
      <div className="relative shrink-0 p-2">
        <Search className="text-muted-foreground absolute top-1/2 left-4 size-3 -translate-y-1/2" />
        <Input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter" className="bg-background/40 pl-6" />
      </div>
    </aside>
  );
}
