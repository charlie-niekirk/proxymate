import type { ReactNode } from "react";

import {
  ContextMenuItem,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import { addFilterTerm } from "@/lib/actions";
import { dslTerm } from "@/lib/copyFormats";

/** What a flow can be filtered by from its menus: null leaves the item out. */
export interface FilterFacts {
  host?: string | null;
  method?: string | null;
  status?: number | null;
  client?: string | null;
}

/** "Filter by ▸ Host, Method, Status, App": narrows the search box (the flow menu and the URL bar). */
export function FilterBySub({ facts, icon }: { facts: FilterFacts; /** For menus whose items all have one. */ icon?: ReactNode }) {
  const items: [string, string, string][] = [];
  if (facts.host) items.push(["Host", facts.host, dslTerm("host", facts.host)]);
  if (facts.method) items.push(["Method", facts.method, dslTerm("method", facts.method)]);
  if (facts.status != null) items.push(["Status", String(facts.status), dslTerm("status", String(facts.status))]);
  if (facts.client) items.push(["App", facts.client, dslTerm("app", facts.client)]);
  if (items.length === 0) return null;
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger data-testid="menu-filter-by">
        {icon}
        Filter by
      </ContextMenuSubTrigger>
      <ContextMenuSubContent className="max-w-80 min-w-44">
        {items.map(([label, value, term]) => (
          <ContextMenuItem key={label} onSelect={() => addFilterTerm(term)} data-testid={`menu-filter-${label.toLowerCase()}`}>
            {label}
            <span className="text-muted-foreground ml-auto truncate pl-3 font-mono">{value}</span>
          </ContextMenuItem>
        ))}
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}
