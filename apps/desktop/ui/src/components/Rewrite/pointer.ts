//! Tree edits addressed by JSON pointer, for `dev/remote.ts` (the bridge's
//! `rewrite.edit`).

import { type Undoable } from "@/hooks/useUndoable";
import { children, insert, nodeAt, parseDoc, remove, rename, setValue, type JNode, type Path } from "@/lib/json/doc";

/** A tree edit addressed by JSON pointer, for dev/remote.ts. */
export type PointerEdit =
  | { set: string; raw: string }
  | { rename: string; key: string }
  | { add: string; key?: string; raw: string }
  | { remove: string };

function pathOf(root: JNode, pointer: string): Path | null {
  const tokens = pointer === "" ? [] : pointer.slice(1).split("/").map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
  const path: number[] = [];
  let node = root;
  for (const t of tokens) {
    const kids = children(node);
    const child = node.t === "obj" ? [...node.entries].reverse().find((e) => e.key === t) : kids[Number(t)];
    if (!child) return null;
    path.push(child.id);
    node = child.v;
  }
  return path;
}

export function editByPointer(tree: Undoable<JNode>, op: PointerEdit) {
  tree.set((root) => {
    if ("set" in op) {
      const path = pathOf(root, op.set);
      return path ? setValue(root, path, parseDoc(op.raw)) : root;
    }
    if ("rename" in op) {
      const path = pathOf(root, op.rename);
      return path ? rename(root, path, op.key) : root;
    }
    if ("remove" in op) {
      const path = pathOf(root, op.remove);
      return path ? remove(root, path) : root;
    }
    const parent = pathOf(root, op.add);
    if (!parent || !nodeAt(root, parent)) return root;
    return insert(root, parent, null, parseDoc(op.raw), op.key).root;
  });
}
