import { Plus, X } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

/** A titled block of a settings page, with an optional control on the right. */
export function Section({
  title,
  description,
  aside,
  children,
  testId,
}: {
  title: string;
  description?: ReactNode;
  aside?: ReactNode;
  children?: ReactNode;
  testId?: string;
}) {
  return (
    <section className="flex flex-col gap-2" data-testid={testId}>
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="text-[13px] font-semibold">{title}</h3>
          {description && <p className="text-muted-foreground mt-0.5 leading-4">{description}</p>}
        </div>
        {aside}
      </div>
      {children}
    </section>
  );
}

/** A settings page's scrolling body. */
export function Page({ children, testId }: { children: ReactNode; testId: string }) {
  return (
    <div className="flex flex-col gap-5 p-4 text-xs" data-testid={testId}>
      {children}
    </div>
  );
}

/** A labelled number input, with its unit and what 0 means. */
export function NumberField({
  label,
  value,
  onChange,
  unit,
  hint,
  min = 0,
  max,
  testId,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  unit?: string;
  hint?: ReactNode;
  min?: number;
  max?: number;
  testId?: string;
}) {
  return (
    <label className="grid grid-cols-[180px_1fr] items-center gap-x-3 gap-y-0.5">
      <span>{label}</span>
      <span className="flex items-center gap-2">
        <Input
          type="number"
          inputMode="numeric"
          min={min}
          max={max}
          value={Number.isFinite(value) ? value : ""}
          onChange={(e) => onChange(e.target.value === "" ? Number.NaN : Number(e.target.value))}
          className="w-28 font-mono tabular-nums"
          data-testid={testId}
        />
        {unit && <span className="text-muted-foreground">{unit}</span>}
      </span>
      {hint && <span className="text-muted-foreground col-start-2 leading-4">{hint}</span>}
    </label>
  );
}

/** An inline error or confirmation under a form. */
export function Note({ error, saved }: { error: string | null; saved?: string | null }) {
  if (error) {
    return (
      <p className="text-[var(--s-failed)]" role="alert" data-testid="settings-error">
        {error}
      </p>
    );
  }
  if (saved) return <p className="text-[var(--s-2xx)]">{saved}</p>;
  return null;
}

/**
 * An editable list of strings (host globs, app ids): add with the box,
 * remove with ×. `onChange` gets the whole new list.
 */
export function ListEditor({
  items,
  onChange,
  placeholder,
  empty,
  mono = true,
  testId,
}: {
  items: string[];
  onChange: (items: string[]) => void;
  placeholder: string;
  empty: string;
  mono?: boolean;
  testId?: string;
}) {
  const [draft, setDraft] = useState("");
  const add = () => {
    const value = draft.trim();
    if (!value || items.includes(value)) return;
    onChange([...items, value]);
    setDraft("");
  };
  return (
    <div className="flex flex-col gap-1.5" data-testid={testId}>
      <ul className="bg-muted/30 max-h-40 overflow-y-auto rounded-md border">
        {items.length === 0 && <li className="text-muted-foreground px-2 py-1.5 italic">{empty}</li>}
        {items.map((item) => (
          <li key={item} className="group flex items-center gap-2 border-b px-2 py-1 last:border-b-0">
            <span className={mono ? "min-w-0 flex-1 truncate font-mono text-[11px]" : "min-w-0 flex-1 truncate"} title={item}>
              {item}
            </span>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label={`Remove ${item}`}
              className="opacity-60 group-hover:opacity-100 focus-visible:opacity-100"
              onClick={() => onChange(items.filter((i) => i !== item))}
            >
              <X />
            </Button>
          </li>
        ))}
      </ul>
      <form
        className="flex gap-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={placeholder}
          spellCheck={false}
          className={mono ? "font-mono" : undefined}
        />
        <Button type="submit" variant="outline" size="sm" disabled={!draft.trim()}>
          <Plus /> Add
        </Button>
      </form>
    </div>
  );
}
