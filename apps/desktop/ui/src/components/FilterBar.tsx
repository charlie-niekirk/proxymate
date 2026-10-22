import { ScanSearch, Search, X } from "lucide-react";
import { useMemo, useRef, useState, type KeyboardEvent } from "react";

import type { ContentGroup, ProtocolFilter } from "@/bindings";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { accept, hasDeepTerms, suggest, termAt, type DslContext, type Suggestion } from "@/lib/dsl";
import { focusFlowTable } from "@/lib/focus";
import { cn } from "@/lib/utils";
import { useSidebar } from "@/store/sidebar";
import { useUi } from "@/store/ui";

const PROTOCOLS: { value: ProtocolFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "http", label: "HTTP" },
  { value: "https", label: "HTTPS" },
  { value: "websocket", label: "WebSocket" },
];

const CONTENT: { value: ContentGroup; label: string }[] = [
  { value: "json", label: "JSON" },
  { value: "form", label: "Form" },
  { value: "xml", label: "XML" },
  { value: "js", label: "JS" },
  { value: "css", label: "CSS" },
  { value: "graphql", label: "GraphQL" },
  { value: "document", label: "Document" },
  { value: "media", label: "Media" },
  { value: "other", label: "Other" },
];

const chip = "h-5 rounded-sm px-2 text-xs data-[state=on]:bg-primary-chip data-[state=on]:text-primary-chip-foreground";

export const SEARCH_INPUT_ID = "flow-search";

/** Filter chips plus the search box. Every change refilters in Rust. */
export function FilterBar() {
  const filter = useUi((s) => s.filter);
  const setProtocol = useUi((s) => s.setProtocol);
  const toggleContent = useUi((s) => s.toggleContent);
  const clearContent = useUi((s) => s.clearContent);

  return (
    <div className="bg-pane-toolbar flex h-8 shrink-0 items-center gap-1 border-b px-2" role="toolbar" aria-label="Filters">
      <ToggleGroup
        type="single"
        value={filter.protocol}
        onValueChange={(v) => v && setProtocol(v as ProtocolFilter)}
        className="gap-0.5"
      >
        {PROTOCOLS.map((p) => (
          <ToggleGroupItem key={p.value} value={p.value} className={chip}>
            {p.label}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      <Separator orientation="vertical" className="mx-1 !h-4" />
      <button
        onClick={clearContent}
        aria-pressed={filter.content.length === 0}
        data-state={filter.content.length === 0 ? "on" : "off"}
        className={cn(chip, "hover:bg-muted text-muted-foreground font-medium")}
      >
        All
      </button>
      {CONTENT.map((c) => (
        <button
          key={c.value}
          onClick={() => toggleContent(c.value)}
          aria-pressed={filter.content.includes(c.value)}
          data-state={filter.content.includes(c.value) ? "on" : "off"}
          className={cn(chip, "hover:bg-muted text-muted-foreground font-medium")}
        >
          {c.label}
        </button>
      ))}
      <SearchBox />
    </div>
  );
}

/**
 * The filter DSL box, with autocomplete for keys and for known hosts, apps
 * and content types. ↑/↓ pick, Tab or Enter accept, Escape closes the list
 * (and then leaves for the table).
 */
function SearchBox() {
  const text = useUi((s) => s.filter.text);
  const setText = useUi((s) => s.setText);
  const deep = useUi((s) => s.deep);
  const snapshot = useSidebar((s) => s.snapshot);
  const inputRef = useRef<HTMLInputElement>(null);
  const [caret, setCaret] = useState(0);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const ctx = useMemo<DslContext>(
    () => ({
      domains: snapshot.domains.filter((d) => d.hosts.length > 0).map((d) => d.domain),
      hosts: snapshot.domains.flatMap((d) => (d.hosts.length > 0 ? d.hosts.map((h) => h.host) : [d.domain])),
      apps: snapshot.apps.map((a) => a.name),
    }),
    [snapshot],
  );
  const suggestions = useMemo(() => (open ? suggest(text, caret, ctx) : []), [open, text, caret, ctx]);
  const shown = suggestions.length > 0;
  const deepQuery = hasDeepTerms(text);

  const syncCaret = () => setCaret(inputRef.current?.selectionStart ?? text.length);

  const choose = (s: Suggestion) => {
    const next = accept(text, caret, s);
    setText(next.text);
    setCaret(next.caret);
    setActive(0);
    setOpen(!s.complete);
    requestAnimationFrame(() => inputRef.current?.setSelectionRange(next.caret, next.caret));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (shown) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = suggestions.length;
        setActive((a) => (a + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        choose(suggestions[Math.min(active, suggestions.length - 1)]!);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setOpen(false);
        return;
      }
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setOpen(true);
      return;
    }
    if (e.key === "Escape" || (e.key === "Enter" && !shown)) {
      e.preventDefault();
      focusFlowTable();
    }
  };

  return (
    <div className="relative ml-auto w-80">
      {deepQuery ? (
        <ScanSearch className="absolute top-1/2 left-2 size-3 -translate-y-1/2 text-[var(--primary-chip-fg)]" />
      ) : (
        <Search className="text-muted-foreground absolute top-1/2 left-2 size-3 -translate-y-1/2" />
      )}
      <Input
        ref={inputRef}
        id={SEARCH_INPUT_ID}
        value={text}
        onChange={(e) => {
          const at = e.target.selectionStart ?? e.target.value.length;
          setText(e.target.value);
          setCaret(at);
          setActive(0);
          // Suggest while a term is being typed, not after a space.
          setOpen(termAt(e.target.value, at).term.length > 0);
        }}
        onKeyDown={onKeyDown}
        onKeyUp={(e) => (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "Home" || e.key === "End") && syncCaret()}
        onClick={syncCaret}
        onBlur={() => setOpen(false)}
        placeholder="Filter: text, host:*.example.com status:4xx body:token…"
        spellCheck={false}
        autoComplete="off"
        role="combobox"
        aria-expanded={shown}
        aria-controls="flow-search-suggestions"
        aria-autocomplete="list"
        aria-activedescendant={shown ? `flow-search-option-${active}` : undefined}
        className="h-6 pr-6 pl-6 font-mono"
        data-testid="flow-search"
      />
      {text && (
        <button
          aria-label="Clear search"
          onClick={() => setText("")}
          className="text-muted-foreground hover:text-foreground absolute top-1/2 right-1.5 -translate-y-1/2"
        >
          <X className="size-3" />
        </button>
      )}
      {deepQuery && deep && <DeepBadge running={deep.running} checked={deep.checked} total={deep.total} />}
      {shown && (
        <ul
          id="flow-search-suggestions"
          role="listbox"
          className="bg-popover text-popover-foreground absolute top-7 right-0 z-50 w-[420px] overflow-hidden rounded-md border p-1 shadow-md"
          data-testid="flow-search-suggestions"
        >
          {suggestions.map((s, i) => (
            <li
              key={s.insert}
              id={`flow-search-option-${i}`}
              role="option"
              aria-selected={i === active}
              onMouseDown={(e) => {
                e.preventDefault();
                choose(s);
              }}
              onMouseEnter={() => setActive(i)}
              className={cn(
                "flex cursor-default items-baseline gap-2 rounded-sm px-2 py-1 text-xs",
                i === active && "bg-accent text-accent-foreground",
              )}
            >
              <span className="shrink-0 font-mono">{s.label}</span>
              {s.detail && <span className="text-muted-foreground truncate">{s.detail}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Inside the box while a deep search runs: how far it got. */
function DeepBadge({ running, checked, total }: { running: boolean; checked: number; total: number }) {
  if (!running) return null;
  const pct = total > 0 ? Math.floor((checked / total) * 100) : 0;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className="text-muted-foreground absolute top-1/2 right-6 -translate-y-1/2 font-mono text-2xs tabular-nums"
          data-testid="deep-badge"
        >
          {pct}%
        </span>
      </TooltipTrigger>
      <TooltipContent>Searching headers and bodies: matches appear as they are found</TooltipContent>
    </Tooltip>
  );
}
