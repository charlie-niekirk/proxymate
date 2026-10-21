import { Search, X } from "lucide-react";
import { useEffect } from "react";

import type { ContentGroup, ProtocolFilter } from "@/bindings";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { cn } from "@/lib/utils";
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

const chip = "h-5 rounded-sm px-2 text-xs data-[state=on]:bg-primary/20 data-[state=on]:text-primary";

export const SEARCH_INPUT_ID = "flow-search";

/** Filter chips plus the search box. Every change refilters in Rust. */
export function FilterBar() {
  const filter = useUi((s) => s.filter);
  const setProtocol = useUi((s) => s.setProtocol);
  const toggleContent = useUi((s) => s.toggleContent);
  const clearContent = useUi((s) => s.clearContent);
  const setText = useUi((s) => s.setText);

  // ⌘F / Ctrl+F focuses the search box.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        document.getElementById(SEARCH_INPUT_ID)?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="bg-pane-toolbar flex h-8 shrink-0 items-center gap-1 border-b px-2">
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
        data-state={filter.content.length === 0 ? "on" : "off"}
        className={cn(chip, "hover:bg-muted text-muted-foreground font-medium")}
      >
        All
      </button>
      {CONTENT.map((c) => (
        <button
          key={c.value}
          onClick={() => toggleContent(c.value)}
          data-state={filter.content.includes(c.value) ? "on" : "off"}
          className={cn(chip, "hover:bg-muted text-muted-foreground font-medium")}
        >
          {c.label}
        </button>
      ))}
      <div className="relative ml-auto w-72">
        <Search className="text-muted-foreground absolute top-1/2 left-2 size-3 -translate-y-1/2" />
        <Input
          id={SEARCH_INPUT_ID}
          value={filter.text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Filter URL (DSL arrives in M10)"
          spellCheck={false}
          className="h-6 pr-6 pl-6 font-mono"
        />
        {filter.text && (
          <button
            aria-label="Clear search"
            onClick={() => setText("")}
            className="text-muted-foreground hover:text-foreground absolute top-1/2 right-1.5 -translate-y-1/2"
          >
            <X className="size-3" />
          </button>
        )}
      </div>
    </div>
  );
}
