import { useContext } from "react";

import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu";
import { copyBodyText } from "@/lib/clipboard";
import { base64 } from "@/lib/copyFormats";

import { BodyCopyContext } from "./bodyContext";
import { CopyItem } from "./items";

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
