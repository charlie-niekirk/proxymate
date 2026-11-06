//! Small pieces of the Rewrite dialog: a scroll pane, a segmented control
//! and the list of actions a save makes.

import { type ReactNode } from "react";

import { actionLabel, actionSummary, type Action } from "@/lib/rules";
import { cn } from "@/lib/utils";

export function Scroll({ children }: { children: ReactNode }) {
  return <div className="h-full overflow-y-auto">{children}</div>;
}

export function Segmented<T extends string>({ value, onChange, options }: { value: T; onChange: (v: T) => void; options: { value: T; label: string; testId: string }[] }) {
  return (
    <div className="bg-muted/50 flex items-center gap-0.5 rounded-md p-0.5 text-xs">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          data-active={o.value === value}
          onClick={() => onChange(o.value)}
          className="text-muted-foreground data-[active=true]:bg-background data-[active=true]:text-foreground h-6 rounded-sm px-2 data-[active=true]:shadow-xs"
          data-testid={o.testId}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function ActionList({ actions, kept, problems }: { actions: Action[]; kept: Action[]; problems: string[] }) {
  return (
    <div className="h-full overflow-y-auto p-2 text-xs" data-testid="rewrite-actions">
      {actions.length === 0 && <div className="text-muted-foreground">Edit the flow to add actions.</div>}
      <ol className="flex flex-col gap-0.5">
        {actions.map((a, i) => (
          <li key={i} className="flex gap-2 font-mono" data-testid="rewrite-action">
            <span className="text-muted-foreground w-28 shrink-0 font-sans">{actionLabel(a.type)}</span>
            <span className="min-w-0 break-all">{actionSummary(a)}</span>
          </li>
        ))}
      </ol>
      {kept.length > 0 && (
        <>
          <div className="text-muted-foreground mt-3 mb-1 text-2xs font-semibold tracking-wide uppercase">Kept from the saved rule</div>
          <ol className={cn("flex flex-col gap-0.5 opacity-80")}>
            {kept.map((a, i) => (
              <li key={i} className="flex gap-2 font-mono" data-testid="rewrite-kept">
                <span className="text-muted-foreground w-28 shrink-0 font-sans">{actionLabel(a.type)}</span>
                <span className="min-w-0 break-all">{actionSummary(a)}</span>
              </li>
            ))}
          </ol>
        </>
      )}
      {problems.length > 0 && (
        <ul className="mt-3 list-disc pl-4 text-[var(--s-failed)]">
          {problems.map((p) => (
            <li key={p}>{p}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
