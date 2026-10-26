import { useState, type ReactElement, type ReactNode } from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { copy, copyFrom, type CopySource } from "@/lib/clipboard";
import { shortcutLabel } from "@/lib/commands";

import { selectionIn } from "./useMenuTarget";

/**
 * "Copy ⌘C" for the text selected when the menu opened. Our menus replace
 * the native one, which would have offered it; nothing when there's no
 * selection.
 */
export function SelectionItem({ text }: { text: string }) {
  if (!text) return null;
  return (
    <>
      <ContextMenuItem onSelect={() => void copy(text, "selection")} data-testid="menu-copy-selection">
        Copy
        <ContextMenuShortcut>{shortcutLabel("c")}</ContextMenuShortcut>
      </ContextMenuItem>
      <ContextMenuSeparator />
    </>
  );
}

/** A menu item that copies `text`; `what` names it in the toast. */
export function CopyItem({
  text,
  what,
  disabled,
  testId,
  children,
}: {
  text: CopySource;
  what?: string;
  disabled?: boolean;
  testId?: string;
  children: ReactNode;
}) {
  return (
    <ContextMenuItem disabled={disabled} onSelect={() => void copyFrom(text, what)} data-testid={testId}>
      {children}
    </ContextMenuItem>
  );
}

export interface CopyEntry {
  label: string;
  text: CopySource;
  what?: string;
  testId?: string;
}

/**
 * A right-click menu of copy items for one element (an address, a path, a
 * pill). `children` must take a ref and props (Radix's `asChild`). `extra`
 * goes after the copies, for the element's own actions.
 */
export function CopyMenu({
  items,
  extra,
  testId,
  children,
}: {
  items: (CopyEntry | null | false | undefined)[];
  extra?: ReactNode;
  testId?: string;
  children: ReactElement;
}) {
  const [selection, setSelection] = useState("");
  const entries = items.filter((e): e is CopyEntry => !!e);
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={(e) => setSelection(selectionIn(e.currentTarget))}>
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent className="max-w-96 min-w-44" data-testid={testId}>
        <SelectionItem text={selection} />
        {entries.map((e) => (
          <CopyItem key={e.label} text={e.text} what={e.what} testId={e.testId}>
            <span className="truncate">{e.label}</span>
          </CopyItem>
        ))}
        {extra && entries.length > 0 && <ContextMenuSeparator />}
        {extra}
      </ContextMenuContent>
    </ContextMenu>
  );
}
