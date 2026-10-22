import { Monitor, Moon, Sun, type LucideIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { usePrefs, type Appearance } from "@/store/prefs";

import { Page, Section } from "./parts";

const OPTIONS: { value: Appearance; label: string; icon: LucideIcon; theme: "light" | "dark" | null }[] = [
  { value: "system", label: "System", icon: Monitor, theme: null },
  { value: "light", label: "Light", icon: Sun, theme: "light" },
  { value: "dark", label: "Dark", icon: Moon, theme: "dark" },
];

/** Settings › Appearance: the theme, and the flow table's column layout. */
export function AppearancePage() {
  const appearance = usePrefs((s) => s.appearance);
  const setAppearance = usePrefs((s) => s.setAppearance);
  const resetColumns = usePrefs((s) => s.resetColumns);
  return (
    <Page testId="settings-appearance">
      <Section title="Theme" description="System follows macOS, switching with it between light and dark.">
        <div role="radiogroup" aria-label="Theme" className="flex gap-3">
          {OPTIONS.map((o) => (
            <button
              key={o.value}
              role="radio"
              aria-checked={appearance === o.value}
              onClick={() => setAppearance(o.value)}
              className={cn(
                "flex w-36 flex-col gap-2 rounded-lg border p-2 text-left",
                appearance === o.value ? "border-primary ring-primary/40 ring-2" : "hover:bg-accent/50",
              )}
              data-testid={`appearance-${o.value}`}
            >
              <ThemeSwatch theme={o.theme} />
              <span className="flex items-center gap-1.5 font-medium">
                <o.icon className="size-3.5" /> {o.label}
              </span>
            </button>
          ))}
        </div>
      </Section>
      <Section
        title="Flow table columns"
        description="Drag a column's edge to resize it, drag its header to move it, and right-click the header to show or hide columns. Widths, order, visibility and sort are remembered."
      >
        <div>
          <Button variant="outline" size="sm" onClick={resetColumns}>
            Reset columns
          </Button>
        </div>
      </Section>
    </Page>
  );
}

/** A tiny window in that theme, drawn with its own tokens (System shows both halves). */
function ThemeSwatch({ theme }: { theme: "light" | "dark" | null }) {
  if (theme === null) {
    return (
      <div className="relative h-16 overflow-hidden rounded-md border">
        <div className="absolute inset-0 [clip-path:polygon(0_0,100%_0,0_100%)]">
          <Mini theme="light" />
        </div>
        <div className="absolute inset-0 [clip-path:polygon(100%_0,100%_100%,0_100%)]">
          <Mini theme="dark" />
        </div>
      </div>
    );
  }
  return (
    <div className="h-16 overflow-hidden rounded-md border">
      <Mini theme={theme} />
    </div>
  );
}

function Mini({ theme }: { theme: "light" | "dark" }) {
  return (
    <div data-theme={theme} className="flex h-full" style={{ background: "var(--pane-table)" }}>
      <div className="w-6" style={{ background: "var(--muted)" }} />
      <div className="flex flex-1 flex-col gap-1 p-1.5">
        <div className="h-1.5 rounded-full" style={{ background: "var(--border)" }} />
        {["--m-get", "--m-post", "--selection-bg", "--m-delete"].map((t) => (
          <div key={t} className="flex items-center gap-1">
            <span className="h-1.5 w-3 rounded-sm" style={{ background: `var(${t})` }} />
            <span className="h-1 flex-1 rounded-full" style={{ background: "var(--muted-foreground)", opacity: 0.4 }} />
          </div>
        ))}
      </div>
    </div>
  );
}
