import { MatcherBuilder } from "@/components/Rules/MatcherBuilder";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { matcherFor, type FlowTarget, type MatchMode } from "@/lib/rewrite/matcher";
import { matcherSummary, type Matcher } from "@/lib/rules";
import { cn } from "@/lib/utils";

export interface MatchChoice {
  /** A chip, or "advanced" for a matcher edited by hand. */
  mode: MatchMode | "advanced";
  anyMethod: boolean;
  matcher: Matcher;
}

const CHIPS: { mode: MatchMode | "advanced"; label: string; title: string }[] = [
  { mode: "exact", label: "Exact URL", title: "This path with these query parameters" },
  { mode: "path", label: "Ignore query", title: "This path, any query string" },
  { mode: "glob", label: "Path glob", title: "Id-like path segments become *" },
  { mode: "advanced", label: "Advanced…", title: "Edit the matcher" },
];

/** Which requests the rewrite applies to: chips, or the full matcher under Advanced. */
export function MatcherChips({ target, choice, onChange }: { target: FlowTarget; choice: MatchChoice; onChange: (c: MatchChoice) => void }) {
  const pick = (mode: MatchMode | "advanced", anyMethod = choice.anyMethod) =>
    onChange({ mode, anyMethod, matcher: mode === "advanced" ? choice.matcher : matcherFor(target, mode, anyMethod) });
  return (
    <div className="flex flex-col gap-1.5 border-b px-3 py-2" data-testid="rewrite-matcher">
      <div className="flex flex-wrap items-center gap-1.5 text-xs">
        <span className="text-muted-foreground mr-1">Applies to</span>
        <ToggleGroup type="single" size="sm" variant="outline" value={choice.mode} onValueChange={(v) => v && pick(v as MatchMode | "advanced")}>
          {CHIPS.map((c) => (
            <ToggleGroupItem key={c.mode} value={c.mode} title={c.title} className="px-2 text-xs" data-testid={`match-${c.mode}`}>
              {c.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <button
          type="button"
          disabled={choice.mode === "advanced"}
          onClick={() => pick(choice.mode, !choice.anyMethod)}
          className={cn(
            "h-6 rounded-md border px-2 disabled:opacity-50",
            choice.anyMethod ? "border-primary bg-primary/15 text-foreground" : "text-muted-foreground",
          )}
          aria-pressed={choice.anyMethod}
          data-testid="match-any-method"
        >
          Any method
        </button>
        <span className="text-muted-foreground ml-auto truncate font-mono text-2xs" data-testid="matcher-summary">
          {matcherSummary(choice.matcher)}
        </span>
      </div>
      {choice.mode === "advanced" && (
        <div className="max-h-56 overflow-y-auto rounded-md border p-2">
          <MatcherBuilder matcher={choice.matcher} onChange={(matcher) => onChange({ ...choice, matcher })} />
        </div>
      )}
    </div>
  );
}
