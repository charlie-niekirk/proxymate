import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  snippetCompletion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { javascript } from "@codemirror/lang-javascript";
import { bracketMatching, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers } from "@codemirror/view";
import { useEffect, useRef } from "react";

import type { ScriptKind } from "@/bindings";
import { codeHighlight, codeTheme } from "@/components/BodyViewer/code/theme";
import { ENTRY_POINTS, GLOBAL_COMPLETIONS, MESSAGE_FIELDS } from "@/lib/scripts";

import type { ScriptEditorProps } from "./ScriptEditor";

/** Completes the script API: globals, their members, message fields, entry points. */
function scriptCompletions(kind: () => ScriptKind) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const member = ctx.matchBefore(/[\w$]+\??\.[\w$]*$/);
    if (member) {
      const object = member.text.slice(0, member.text.indexOf(".")).replace(/\?$/, "");
      const global = GLOBAL_COMPLETIONS[object];
      const options = global
        ? global.members.map((m) => ({ label: m.label, detail: m.detail, type: "function" }))
        : MESSAGE_FIELDS.map((f) => ({ label: f.label, detail: f.detail, type: "property" }));
      return { from: member.from + member.text.indexOf(".") + 1, options, validFor: /^[\w$]*$/ };
    }
    const word = ctx.matchBefore(/[\w$]+$/);
    if (!word && !ctx.explicit) return null;
    return {
      from: word?.from ?? ctx.pos,
      options: [
        ...Object.entries(GLOBAL_COMPLETIONS).map(([label, g]) => ({ label, detail: g.detail, type: "variable" })),
        ...ENTRY_POINTS[kind()].map((e) =>
          snippetCompletion(e.snippet, { label: e.label, detail: "entry point", type: "function" }),
        ),
      ],
      validFor: /^[\w$]*$/,
    };
  };
}

/** The CodeMirror behind `ScriptEditor` (loaded lazily). */
export function ScriptCodeMirror({ value, kind, onChange, testId = "script-editor" }: ScriptEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const changed = useRef(onChange);
  const currentKind = useRef(kind);
  const completions = useRef(new Compartment());
  useEffect(() => {
    changed.current = onChange;
    currentKind.current = kind;
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
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          highlightSelectionMatches(),
          search({ top: true }),
          javascript(),
          completions.current.of(
            autocompletion({ override: [scriptCompletions(() => currentKind.current)], icons: false }),
          ),
          keymap.of([
            ...closeBracketsKeymap,
            ...completionKeymap,
            ...searchKeymap,
            ...historyKeymap,
            indentWithTab,
            ...defaultKeymap,
          ]),
          syntaxHighlighting(codeHighlight),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) changed.current(u.state.doc.toString());
          }),
          codeTheme,
        ],
      }),
    });
    return () => editor.destroy();
    // Seeded once; later edits live in the editor (remount with a key to reset).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return <div ref={host} className="h-full min-h-0" data-selectable data-testid={testId} />;
}
