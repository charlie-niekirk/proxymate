import { useQuery } from "@tanstack/react-query";
import { PencilLine } from "lucide-react";

import { Centered } from "@/components/BodyViewer/parts";
import { keepEscapeInEditor } from "@/components/JsonEditor/escape";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useUi } from "@/store/ui";

import { rewriteKey } from "@/lib/query";
import { errorText, load } from "./load";
import { RewriteEditor } from "./RewriteEditor";

/**
 * The Rewrite dialog: a flow's request and response as editable tables and
 * trees. Saving turns the edits into a rule that patches only what changed
 * (`lib/rewrite/diff.ts`), previewed live with the engine's own dry run.
 * Opens from "Rewrite…" on a flow (live or a document), the inspector's
 * URL bar, and "Edit visually" in the Rules Editor.
 */
export function RewriteDialog() {
  const { open, store, flow, rule, seq } = useUi((s) => s.rewrite);
  const close = useUi((s) => s.closeRewrite);
  const loaded = useQuery({
    queryKey: rewriteKey(seq),
    enabled: open && flow !== null,
    queryFn: () => load(store, flow!, rule),
    gcTime: 0,
    staleTime: Infinity,
    retry: false,
  });
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="flex h-[88vh] flex-col gap-0 p-0 sm:max-w-[1320px]"
        // The dialog itself, not its first field: the editors take keys.
        initialFocus="content"
        onEscapeKeyDown={keepEscapeInEditor}
        data-testid="rewrite-dialog"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <PencilLine className="size-4" />
            {rule?.id ? "Edit rewrite" : "Rewrite"}
          </DialogTitle>
          <DialogDescription>
            Edit the flow as it should be. Saving makes a rule that changes only what you changed, on every request it
            matches.
          </DialogDescription>
        </DialogHeader>
        {loaded.isPending ? (
          <Centered>Loading the flow…</Centered>
        ) : loaded.isError ? (
          <Centered>Can&apos;t edit this flow: {errorText(loaded.error)}</Centered>
        ) : (
          <RewriteEditor key={seq} loaded={loaded.data} onDone={close} />
        )}
      </DialogContent>
    </Dialog>
  );
}
