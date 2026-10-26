import { lazy, Suspense, useRef, useState, type ReactNode } from "react";

import { BodyItems } from "@/components/menus/body";
import { CopyItem, SelectionItem } from "@/components/menus/items";
import { selectionIn } from "@/components/menus/useMenuTarget";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { shortcutLabel } from "@/lib/commands";
import { cn } from "@/lib/utils";

import type { CodeLanguage } from "./code/languages";
import { loadCodeView } from "./code/load";

export interface CodeViewProps {
  text: string;
  language: CodeLanguage;
  highlight?: boolean;
  wrap?: boolean;
  className?: string;
  /** The editor's commands, for the context menu (set once CodeMirror has loaded). */
  onHandle?: (handle: CodeHandle | null) => void;
}

/** What the context menu asks of the loaded editor. */
export interface CodeHandle {
  /** The selected text (every range, a line apart). */
  selection(): string;
  selectAll(): void;
  find(): void;
}

const CodeMirrorView = lazy(() => loadCodeView().then((m) => ({ default: m.CodeMirrorView })));

/**
 * Read-only CodeMirror 6: line numbers, folding, bracket matching and
 * in-body search (⌘F). Only visible lines are rendered, so multi-megabyte
 * bodies scroll smoothly; `highlight={false}` also skips parsing them.
 * CodeMirror and each grammar load on first use (`code/CodeMirrorView`),
 * keeping them out of the startup bundle; until then the space stays empty.
 * The context menu (outside the lazy part, so it works while that loads)
 * copies the selection or everything shown, then the body; `menuItems`
 * adds a viewer's own (Raw: the start line, the headers).
 */
export function CodeView({ menuItems, className, ...props }: Omit<CodeViewProps, "onHandle"> & { menuItems?: ReactNode }) {
  const handle = useRef<CodeHandle | null>(null);
  const [selection, setSelection] = useState("");
  const onContextMenu = (e: React.MouseEvent) => {
    // WebKit selects the word under the cursor just before this event, and
    // CodeMirror only hears about it later: trust the DOM unless CodeMirror's
    // own selection (which may span lines that aren't rendered) contains it.
    const dom = selectionIn(e.currentTarget);
    const editor = handle.current?.selection() ?? "";
    setSelection(editor && editor.includes(dom) ? editor : dom || editor);
  };
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={onContextMenu}>
        <div className={cn("h-full min-h-0", className)}>
          <Suspense fallback={<div className="h-full min-h-0" data-testid="code-view-loading" />}>
            <CodeMirrorView {...props} onHandle={(h) => (handle.current = h)} />
          </Suspense>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-48" data-testid="code-menu">
        <SelectionItem text={selection} />
        <CopyItem text={props.text} what="text" testId="code-copy-all">
          Copy all
        </CopyItem>
        <ContextMenuItem onSelect={() => handle.current?.selectAll()}>
          Select all
          <ContextMenuShortcut>{shortcutLabel("a")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => handle.current?.find()}>
          Find…
          <ContextMenuShortcut>{shortcutLabel("f")}</ContextMenuShortcut>
        </ContextMenuItem>
        {menuItems && (
          <>
            <ContextMenuSeparator />
            {menuItems}
          </>
        )}
        <BodyItems />
      </ContextMenuContent>
    </ContextMenu>
  );
}
