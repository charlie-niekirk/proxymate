import { lazy, Suspense } from "react";

import { cn } from "@/lib/utils";

import type { CodeLanguage } from "./code/languages";
import { loadCodeView } from "./code/load";

export interface CodeViewProps {
  text: string;
  language: CodeLanguage;
  highlight?: boolean;
  wrap?: boolean;
  className?: string;
}

const CodeMirrorView = lazy(() => loadCodeView().then((m) => ({ default: m.CodeMirrorView })));

/**
 * Read-only CodeMirror 6: line numbers, folding, bracket matching and
 * in-body search (⌘F). Only visible lines are rendered, so multi-megabyte
 * bodies scroll smoothly; `highlight={false}` also skips parsing them.
 * CodeMirror and each grammar load on first use (`code/CodeMirrorView`),
 * keeping them out of the startup bundle; until then the space stays empty.
 */
export function CodeView(props: CodeViewProps) {
  return (
    <Suspense fallback={<div className={cn("h-full min-h-0", props.className)} data-testid="code-view-loading" />}>
      <CodeMirrorView {...props} />
    </Suspense>
  );
}
