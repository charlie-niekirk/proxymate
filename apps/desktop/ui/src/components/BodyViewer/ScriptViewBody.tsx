import { useEffect, useMemo } from "react";

import { scriptViewBytes, scriptViewMime, viewerFor, type ScriptViewOption } from "@/lib/scripts";

import { HIGHLIGHT_LIMIT } from "./util";
import { Viewer } from "./Viewer";

/** A view a script produced, through the built-in viewer for its kind. */
export function ScriptViewBody({ option, bodyId }: { option: ScriptViewOption; bodyId: string }) {
  const { view } = option;
  const bytes = useMemo(() => scriptViewBytes(view), [view]);
  const mime = scriptViewMime(view);
  const viewer = viewerFor(view.kind);
  const url = useMemo(
    () => (viewer === "image" ? URL.createObjectURL(new Blob([bytes.slice()], { type: mime })) : null),
    [viewer, bytes, mime],
  );
  useEffect(() => () => (url ? URL.revokeObjectURL(url) : undefined), [url]);
  return (
    <div className="h-full" data-testid="script-view" data-script-kind={view.kind}>
      <Viewer
        viewer={viewer}
        bytes={bytes}
        url={url}
        mime={mime}
        size={bytes.length}
        contentType={mime}
        highlight={bytes.length <= HIGHLIGHT_LIMIT}
        bodyId={`${bodyId}-${option.value}`}
      />
    </div>
  );
}
