import { defaultKeymap } from "@codemirror/commands";
import { bracketMatching, codeFolding, foldGutter, foldKeymap, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, openSearchPanel, search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers } from "@codemirror/view";
import { useEffect, useRef } from "react";

import { cn } from "@/lib/utils";

import type { CodeViewProps } from "../CodeView";
import { applyLanguage, readyLanguage } from "./languages";
import { codeHighlight, codeTheme } from "./theme";

/**
 * The CodeMirror behind `CodeView` (which loads this module lazily).
 * The fold gutter is there from the start and the grammar is swapped in
 * when it has loaded, so the layout doesn't move when highlighting arrives.
 */
export function CodeMirrorView({ text, language, highlight = true, wrap = false, className, onHandle }: CodeViewProps) {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const lang = new Compartment();
    const highlighted = highlight && language !== "text";
    const extensions: Extension[] = [
      EditorState.readOnly.of(true),
      lineNumbers(),
      drawSelection(),
      highlightSelectionMatches(),
      search({ top: true }),
      keymap.of([...searchKeymap, ...foldKeymap, ...defaultKeymap]),
      codeTheme,
    ];
    if (highlighted) {
      extensions.push(lang.of(readyLanguage(language) ?? []), syntaxHighlighting(codeHighlight), codeFolding(), foldGutter(), bracketMatching());
    }
    if (wrap) extensions.push(EditorView.lineWrapping);
    const view = new EditorView({ parent, state: EditorState.create({ doc: text, extensions }) });
    const cancel = highlighted ? applyLanguage(view, lang, language) : () => {};
    onHandle?.({
      selection: () => view.state.selection.ranges.map((r) => view.state.sliceDoc(r.from, r.to)).filter(Boolean).join("\n"),
      selectAll: () => {
        view.dispatch({ selection: { anchor: 0, head: view.state.doc.length } });
        view.focus();
      },
      find: () => {
        openSearchPanel(view);
      },
    });
    return () => {
      onHandle?.(null);
      cancel();
      view.destroy();
    };
    // onHandle is a fresh closure each render; the handle only changes with the view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, language, highlight, wrap]);

  return <div ref={host} className={cn("h-full min-h-0", className)} data-selectable data-testid="code-view" data-language={language} />;
}
