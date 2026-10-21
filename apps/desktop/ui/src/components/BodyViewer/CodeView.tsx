import { defaultKeymap } from "@codemirror/commands";
import { bracketMatching, codeFolding, foldGutter, foldKeymap, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers } from "@codemirror/view";
import { useEffect, useRef } from "react";

import { cn } from "@/lib/utils";

import { languageExtension, type CodeLanguage } from "./code/languages";
import { codeHighlight, codeTheme } from "./code/theme";

/**
 * Read-only CodeMirror 6: line numbers, folding, bracket matching and
 * in-body search (⌘F). Only visible lines are rendered, so multi-megabyte
 * bodies scroll smoothly; `highlight={false}` also skips parsing them.
 */
export function CodeView({
  text,
  language,
  highlight = true,
  wrap = false,
  className,
}: {
  text: string;
  language: CodeLanguage;
  highlight?: boolean;
  wrap?: boolean;
  className?: string;
}) {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const extensions: Extension[] = [
      EditorState.readOnly.of(true),
      lineNumbers(),
      drawSelection(),
      highlightSelectionMatches(),
      search({ top: true }),
      keymap.of([...searchKeymap, ...foldKeymap, ...defaultKeymap]),
      codeTheme,
    ];
    if (highlight && language !== "text") {
      extensions.push(languageExtension(language), syntaxHighlighting(codeHighlight), codeFolding(), foldGutter(), bracketMatching());
    }
    if (wrap) extensions.push(EditorView.lineWrapping);
    const view = new EditorView({ parent, state: EditorState.create({ doc: text, extensions }) });
    return () => view.destroy();
  }, [text, language, highlight, wrap]);

  return <div ref={host} className={cn("h-full min-h-0", className)} data-selectable data-testid="code-view" data-language={language} />;
}
