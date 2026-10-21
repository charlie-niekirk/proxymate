import { CaseSensitive } from "lucide-react";
import type { ReactNode, TextareaHTMLAttributes } from "react";

import type { PatternKind, Phase } from "@/bindings";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Toggle } from "@/components/ui/toggle";
import { newPattern, type Pattern } from "@/lib/rules";
import { cn } from "@/lib/utils";

/** A labelled row in the rule forms. */
export function Field({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-w-0 items-center gap-2", className)}>
      <span className="text-muted-foreground w-24 shrink-0 text-right text-xs">{label}</span>
      <div className="flex min-w-0 flex-1 items-center gap-1.5">{children}</div>
    </div>
  );
}

export function Checkbox({
  checked,
  onChange,
  label,
  testId,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  testId?: string;
}) {
  return (
    <label className="inline-flex cursor-default items-center gap-1.5 text-xs select-none">
      <input
        type="checkbox"
        className="size-3.5 accent-[var(--primary)]"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        data-testid={testId}
      />
      {label}
    </label>
  );
}

export function TextArea({ className, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <textarea
      spellCheck={false}
      className={cn(
        "border-input dark:bg-input/30 min-h-16 w-full resize-y rounded-md border bg-transparent px-2 py-1 font-mono text-xs outline-none",
        "focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]",
        className,
      )}
      {...props}
    />
  );
}

/** A compact select over a fixed list of options. */
export function Choice<T extends string>({
  value,
  options,
  onChange,
  className,
  testId,
}: {
  value: T;
  options: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
  className?: string;
  testId?: string;
}) {
  return (
    <Select value={value} onValueChange={(v) => onChange(v as T)}>
      <SelectTrigger className={cn("shrink-0", className)} data-testid={testId}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value} className="text-xs">
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

const PHASES = [
  { value: "request", label: "Request" },
  { value: "response", label: "Response" },
] as const;

export function PhaseChoice({ value, onChange }: { value: Phase; onChange: (phase: Phase) => void }) {
  return <Choice value={value} options={PHASES} onChange={onChange} className="w-24" />;
}

const KINDS = [
  { value: "exact", label: "Exact" },
  { value: "glob", label: "Glob" },
  { value: "regex", label: "Regex" },
] as const;

/**
 * A pattern: kind, value and a case-insensitivity toggle. With `optional`,
 * an empty value means "any" (null).
 */
export function PatternInput({
  value,
  onChange,
  placeholder,
  optional = false,
  testId,
}: {
  value: Pattern | null;
  onChange: (pattern: Pattern | null) => void;
  placeholder?: string;
  optional?: boolean;
  testId?: string;
}) {
  const pattern = value ?? newPattern();
  const update = (next: Partial<Pattern>) => {
    const merged = { ...pattern, ...next };
    onChange(optional && merged.value === "" ? null : merged);
  };
  return (
    <div className="flex min-w-0 flex-1 items-center gap-1">
      <Choice<PatternKind>
        value={pattern.kind}
        options={KINDS}
        onChange={(kind) => update({ kind })}
        className="w-[72px]"
      />
      <Input
        value={pattern.value}
        placeholder={placeholder ?? (optional ? "any" : "")}
        onChange={(e) => update({ value: e.target.value })}
        className="h-6 font-mono text-xs"
        data-testid={testId}
      />
      <Toggle
        size="sm"
        aria-label="Case-insensitive"
        title="Case-insensitive"
        pressed={pattern.case_insensitive}
        onPressedChange={(case_insensitive) => update({ case_insensitive })}
        className="h-6 min-w-6 px-1"
      >
        <CaseSensitive className="size-3.5" />
      </Toggle>
    </div>
  );
}

/** A number input that reports integers (empty → `fallback`). */
export function NumberInput({
  value,
  onChange,
  min = 0,
  max,
  className,
  placeholder,
}: {
  value: number | null;
  onChange: (value: number | null) => void;
  min?: number;
  max?: number;
  className?: string;
  placeholder?: string;
}) {
  return (
    <Input
      type="number"
      min={min}
      max={max}
      value={value ?? ""}
      placeholder={placeholder}
      onChange={(e) => onChange(e.target.value === "" ? null : Math.trunc(Number(e.target.value)))}
      className={cn("h-6 w-24 font-mono text-xs", className)}
    />
  );
}
