import { lazy, Suspense } from "react";

import type { CodeLanguage } from "@/components/BodyViewer/code/languages";
import { loadCodeEditor } from "@/components/BodyViewer/code/load";
import { cn } from "@/lib/utils";

export interface CodeEditorProps {
  /** Seeds the editor on mount; later changes to it are ignored (remount with a `key` to reset). */
  value: string;
  language: CodeLanguage;
  onChange?: (text: string) => void;
  readOnly?: boolean;
  className?: string;
  testId?: string;
}

const CodeMirrorEditor = lazy(() => loadCodeEditor().then((m) => ({ default: m.CodeMirrorEditor })));

/**
 * An editable CodeMirror 6 for message bodies (Compose, the Breakpoint
 * Editor), highlighted as `language`. Uncontrolled after mount: `value`
 * seeds it, `onChange` hears every edit. CodeMirror and the grammar load on
 * first use; until then the space stays empty.
 */
export function CodeEditor(props: CodeEditorProps) {
  return (
    <Suspense fallback={<div className={cn("h-full min-h-0", props.className)} data-testid={`${props.testId ?? "code-editor"}-loading`} />}>
      <CodeMirrorEditor {...props} />
    </Suspense>
  );
}
