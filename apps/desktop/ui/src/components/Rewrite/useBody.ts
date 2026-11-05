//! One side's body state in the Rewrite dialog.

import { useMemo, useState } from "react";

import { useUndoable } from "@/hooks/useUndoable";
import type { JNode } from "@/lib/json/doc";
import type { KvRow } from "@/lib/rewrite/diff";
import type { BodyEdit } from "@/lib/rewrite/session";

/** One side's body: every kind's state (hooks can't be conditional), and the edit it adds up to. */
export function useBody(initial: BodyEdit) {
  const tree = useUndoable<JNode>(() => (initial.kind === "json" ? initial.doc : initial.kind === "graphql" ? initial.vars : { t: "null" }));
  const [whole, setWhole] = useState(initial.kind === "json" || initial.kind === "graphql" || initial.kind === "form" ? initial.whole : false);
  const [rows, setRows] = useState<KvRow[]>(initial.kind === "form" ? initial.rows : []);
  const [text, setText] = useState(initial.kind === "text" ? initial.text : "");
  const [mode, setMode] = useState<"replace" | "find">(initial.kind === "text" ? initial.mode : "find");
  const [pairs, setPairs] = useState(initial.kind === "text" ? initial.pairs : []);
  const edit = useMemo((): BodyEdit => {
    switch (initial.kind) {
      case "json":
        return { kind: "json", doc: tree.value, whole };
      case "graphql":
        return { kind: "graphql", vars: tree.value, whole };
      case "form":
        return { kind: "form", rows, whole };
      case "text":
        return { kind: "text", text, mode, pairs };
      case "none":
        return initial;
    }
  }, [initial, tree.value, whole, rows, text, mode, pairs]);
  return { tree, whole, setWhole, rows, setRows, text, setText, mode, setMode, pairs, setPairs, edit };
}

export type BodyState = ReturnType<typeof useBody>;
