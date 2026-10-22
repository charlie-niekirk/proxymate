import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers } from "@codemirror/view";
import { useEffect, useRef } from "react";

import { applyLanguage, readyLanguage } from "@/components/BodyViewer/code/languages";
import { codeHighlight, codeTheme } from "@/components/BodyViewer/code/theme";
import { cn } from "@/lib/utils";

import type { CodeEditorProps } from "./CodeEditor";

/** The CodeMirror behind `CodeEditor` (which loads this module lazily). */
export function CodeMirrorEditor({ value, language, onChange, readOnly = false, className, testId = "code-editor" }: CodeEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const lang = useRef(new Compartment());
  const editable = useRef(new Compartment());
  const changed = useRef(onChange);
  useEffect(() => {
    changed.current = onChange;
  });

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const editor = new EditorView({
      parent,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          history(),
          drawSelection(),
          bracketMatching(),
          highlightSelectionMatches(),
          search({ top: true }),
          keymap.of([...searchKeymap, ...historyKeymap, indentWithTab, ...defaultKeymap]),
          syntaxHighlighting(codeHighlight),
          lang.current.of(readyLanguage(language) ?? []),
          editable.current.of(readOnlyExtension(readOnly)),
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) changed.current?.(u.state.doc.toString());
          }),
          codeTheme,
        ],
      }),
    });
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
    // Seeded once; later edits live in the editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The language follows the prop (a Content-Type change); grammars load on demand.
  useEffect(() => (view.current ? applyLanguage(view.current, lang.current, language) : undefined), [language]);

  useEffect(() => {
    view.current?.dispatch({ effects: editable.current.reconfigure(readOnlyExtension(readOnly)) });
  }, [readOnly]);

  return (
    <div
      ref={host}
      className={cn("h-full min-h-0", readOnly && "opacity-70", className)}
      data-selectable
      data-testid={testId}
      data-language={language}
      data-readonly={readOnly}
    />
  );
}

function readOnlyExtension(readOnly: boolean) {
  return readOnly ? [EditorState.readOnly.of(true), EditorView.editable.of(false)] : [];
}
