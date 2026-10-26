import { useContext, useState, type ReactElement } from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { copyBodyText } from "@/lib/clipboard";
import { base64 } from "@/lib/copyFormats";

import { BodyCopyContext } from "./bodyContext";
import { CopyItem, SelectionItem } from "./items";
import { selectionIn } from "./useMenuTarget";

/** Same heuristic as `bodyText`: a NUL near the start means binary. */
const looksBinary = (bytes: Uint8Array) => bytes.subarray(0, 8192).includes(0);

/** Copy body, Copy body as Base64, Save body…: the end of every body viewer's menu. */
export function BodyItems({ separator = true }: { separator?: boolean }) {
  const body = useContext(BodyCopyContext);
  if (!body?.bytes || body.bytes.length === 0) return null;
  const { bytes, what, onSave } = body;
  return (
    <>
      {separator && <ContextMenuSeparator />}
      <ContextMenuItem disabled={looksBinary(bytes)} onSelect={() => void copyBodyText(bytes, what)} data-testid="menu-copy-body">
        Copy {what}
      </ContextMenuItem>
      <CopyItem text={async () => base64(bytes)} what={`${what} as Base64`} testId="menu-copy-body-base64">
        Copy {what} as Base64
      </CopyItem>
      {onSave && <ContextMenuItem onSelect={onSave}>Save {what}…</ContextMenuItem>}
    </>
  );
}

/**
 * The menu for viewers without one of their own (fonts, media, multipart
 * cards…): the selection, then the body items. Viewers with their own menu
 * (code, JSON tree, hex, tables) open theirs instead, which end the same way.
 */
export function BodyFallbackMenu({ children }: { children: ReactElement }) {
  const body = useContext(BodyCopyContext);
  const [selection, setSelection] = useState("");
  const onContextMenu = (e: React.MouseEvent) => {
    const text = selectionIn(e.currentTarget);
    if (!text && !body?.bytes?.length) {
      e.preventDefault();
      return;
    }
    setSelection(text);
  };
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={onContextMenu}>
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-48" data-testid="body-menu">
        <SelectionItem text={selection} />
        <BodyItems separator={false} />
      </ContextMenuContent>
    </ContextMenu>
  );
}
