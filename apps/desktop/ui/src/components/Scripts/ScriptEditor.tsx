import { lazy, Suspense } from "react";

import type { ScriptKind } from "@/bindings";

export interface ScriptEditorProps {
  /** Seeds the editor on mount (remount with a `key` to reset). */
  value: string;
  /** Which entry points to complete. */
  kind: ScriptKind;
  onChange: (source: string) => void;
  testId?: string;
}

const ScriptCodeMirror = lazy(() => import("./ScriptCodeMirror").then((m) => ({ default: m.ScriptCodeMirror })));

/**
 * A JavaScript editor for scripts, with completions for the script API
 * (`lib/scripts.ts`). CodeMirror loads on first use.
 */
export function ScriptEditor(props: ScriptEditorProps) {
  return (
    <Suspense fallback={<div className="h-full min-h-0" data-testid="script-editor-loading" />}>
      <ScriptCodeMirror {...props} />
    </Suspense>
  );
}
