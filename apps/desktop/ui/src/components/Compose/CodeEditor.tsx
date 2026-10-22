import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers } from "@codemirror/view";
import { useEffect, useRef } from "react";

import { languageExtension, type CodeLanguage } from "@/components/BodyViewer/code/languages";
import { codeHighlight, codeTheme } from "@/components/BodyViewer/code/theme";
import { cn } from "@/lib/utils";

/**
 * An editable CodeMirror 6 for request bodies, highlighted as `language`.
 * Uncontrolled after mount: `value` seeds it, `onChange` hears every edit.
 */
export function CodeEditor({
  value,
  language,
  onChange,
  className,
}: {
  value: string;
  language: CodeLanguage;
  onChange: (text: string) => void;
  className?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const lang = useRef(new Compartment());
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
          lang.current.of(language === "text" ? [] : languageExtension(language)),
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) changed.current(u.state.doc.toString());
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

  useEffect(() => {
    view.current?.dispatch({
      effects: lang.current.reconfigure(language === "text" ? [] : languageExtension(language)),
    });
  }, [language]);

  return <div ref={host} className={cn("h-full min-h-0", className)} data-selectable data-testid="code-editor" />;
}
