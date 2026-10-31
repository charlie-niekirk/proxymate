import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { BodyPart } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { commentFlow, errorText } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { flowKey } from "@/lib/query";
import { MAX_HEADER_COLUMNS, usePrefs } from "@/store/prefs";
import { useUi } from "@/store/ui";

/** "Comment…": a flow's note, kept with it in saved sessions and HAR exports. */
export function CommentDialog() {
  const { open, store, flow } = useUi((s) => s.comment);
  const close = useUi((s) => s.closeComment);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-[440px]" data-testid="comment-dialog">
        <DialogHeader>
          <DialogTitle className="text-sm">Comment</DialogTitle>
          <DialogDescription>Saved with the flow in sessions and HAR exports. Find commented flows with is:commented.</DialogDescription>
        </DialogHeader>
        {flow !== null && <CommentForm key={`${store}:${flow}`} store={store} flow={flow} onDone={close} />}
      </DialogContent>
    </Dialog>
  );
}

function CommentForm({ store, flow, onDone }: { store: number; flow: number; onDone: () => void }) {
  const { data: summary, isPending } = useQuery({ queryKey: [...flowKey(store, flow), "summary"], queryFn: () => backend.getFlow(store, flow) });
  if (isPending) return null;
  return <CommentEditor store={store} flow={flow} initial={summary?.comment ?? ""} onDone={onDone} />;
}

function CommentEditor({ store, flow, initial, onDone }: { store: number; flow: number; initial: string; onDone: () => void }) {
  const [text, setText] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const save = async (value: string) => {
    try {
      await commentFlow(store, flow, value);
      onDone();
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void save(text);
      }}
      className="flex flex-col gap-3"
    >
      <textarea
        autoFocus
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void save(text);
        }}
        rows={5}
        className="border-input bg-background focus-visible:ring-ring/50 w-full resize-none rounded-md border px-2 py-1.5 text-[13px] outline-none focus-visible:ring-2"
        placeholder="What's special about this flow?"
        aria-label="Comment"
        data-testid="comment-text"
      />
      {error && <p className="text-destructive text-xs">{error}</p>}
      <DialogFooter>
        {initial && (
          <Button type="button" variant="ghost" size="sm" className="mr-auto" onClick={() => void save("")} data-testid="comment-remove">
            Remove
          </Button>
        )}
        <Button type="button" variant="outline" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button type="submit" size="sm" data-testid="comment-save">
          Save
        </Button>
      </DialogFooter>
    </form>
  );
}

/** "Add header column…": a request or response header shown as a flow table column. */
export function AddColumnDialog() {
  const open = useUi((s) => s.addColumnOpen);
  const setOpen = useUi((s) => s.setAddColumnOpen);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="sm:max-w-[400px]" data-testid="add-column-dialog">
        <DialogHeader>
          <DialogTitle className="text-sm">Add header column</DialogTitle>
          <DialogDescription>
            Shows a header's value for every flow, existing ones included. Up to {MAX_HEADER_COLUMNS} columns.
          </DialogDescription>
        </DialogHeader>
        {open && <AddColumnForm onDone={() => setOpen(false)} />}
      </DialogContent>
    </Dialog>
  );
}

function AddColumnForm({ onDone }: { onDone: () => void }) {
  const add = usePrefs((s) => s.addHeaderColumn);
  const [part, setPart] = useState<BodyPart>("request");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const submit = () => {
    const trimmed = name.trim();
    if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(trimmed)) {
      setError("Enter a header name, like x-request-id");
      return;
    }
    if (!add({ part, name: trimmed })) {
      setError(`That column is already there, or there are ${MAX_HEADER_COLUMNS} already`);
      return;
    }
    onDone();
  };
  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <div className="flex gap-2">
        <Select value={part} onValueChange={(v) => setPart(v as BodyPart)}>
          <SelectTrigger className="h-7 w-32 text-xs" aria-label="Request or response" data-testid="add-column-part">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="request">Request</SelectItem>
            <SelectItem value="response">Response</SelectItem>
          </SelectContent>
        </Select>
        <Input
          autoFocus
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            setError(null);
          }}
          placeholder="x-request-id"
          className="h-7 font-mono text-xs"
          aria-label="Header name"
          data-testid="add-column-name"
        />
      </div>
      {error && <p className="text-destructive text-xs">{error}</p>}
      <DialogFooter>
        <Button type="button" variant="outline" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button type="submit" size="sm" data-testid="add-column-save">
          Add Column
        </Button>
      </DialogFooter>
    </form>
  );
}
