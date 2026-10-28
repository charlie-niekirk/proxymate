import { defaultKeymap } from "@codemirror/commands";
import { bracketMatching, codeFolding, foldGutter, foldKeymap, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, openSearchPanel, search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  MatchDecorator,
  ViewPlugin,
  drawSelection,
  keymap,
  lineNumbers,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/utils";

import type { CodeViewProps } from "../CodeView";
import { MatchBar } from "../search";
import { matchRanges, searchRegExp } from "../searchTerms";
import { applyLanguage, readyLanguage } from "./languages";
import { codeHighlight, codeTheme } from "./theme";

/**
 * The CodeMirror behind `CodeView` (which loads this module lazily).
 * The fold gutter is there from the start and the grammar is swapped in
 * when it has loaded, so the layout doesn't move when highlighting arrives.
 */
const filterMark = Decoration.mark({ class: "cm-pm-filter-match" });

/** Marks every match of `re` in the visible lines. */
function filterMatches(re: RegExp | null): Extension {
  if (!re) return [];
  const decorator = new MatchDecorator({ regexp: new RegExp(re.source, "gi"), decoration: filterMark, maxLength: 2000 });
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = decorator.createDeco(view);
      }
      update(update: ViewUpdate) {
        this.decorations = decorator.updateDeco(update, this.decorations);
      }
    },
    { decorations: (v) => v.decorations },
  );
}

export function CodeMirrorView({ text, language, highlight = true, wrap = false, className, onHandle, matches = [] }: CodeViewProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const marks = useRef(new Compartment());
  const termsKey = matches.join("\u0000");
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed by the terms' text, not the array's identity.
  const re = useMemo(() => searchRegExp(matches), [termsKey]);
  const ranges = useMemo(() => (re ? matchRanges(text, re) : []), [re, text]);
  // The match last stepped to, for these ranges (new ones start over).
  const [stepped, setStepped] = useState<{ ranges: [number, number][]; index: number } | null>(null);
  const current = stepped?.ranges === ranges ? stepped.index : -1;

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
      marks.current.of(filterMatches(re)),
    ];
    if (highlighted) {
      extensions.push(lang.of(readyLanguage(language) ?? []), syntaxHighlighting(codeHighlight), codeFolding(), foldGutter(), bracketMatching());
    }
    if (wrap) extensions.push(EditorView.lineWrapping);
    const editor = new EditorView({ parent, state: EditorState.create({ doc: text, extensions }) });
    view.current = editor;
    const cancel = highlighted ? applyLanguage(editor, lang, language) : () => {};
    onHandle?.({
      selection: () => editor.state.selection.ranges.map((r) => editor.state.sliceDoc(r.from, r.to)).filter(Boolean).join("\n"),
      selectAll: () => {
        editor.dispatch({ selection: { anchor: 0, head: editor.state.doc.length } });
        editor.focus();
      },
      find: () => {
        openSearchPanel(editor);
      },
    });
    return () => {
      onHandle?.(null);
      cancel();
      view.current = null;
      editor.destroy();
    };
    // onHandle is a fresh closure each render; the handle only changes with the view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, language, highlight, wrap]);

  // New terms re-mark the open editor; no need to rebuild it.
  useEffect(() => {
    view.current?.dispatch({ effects: marks.current.reconfigure(filterMatches(re)) });
  }, [re]);

  const step = (direction: 1 | -1) => {
    const editor = view.current;
    if (!editor || ranges.length === 0) return;
    const next = current < 0 ? (direction === 1 ? 0 : ranges.length - 1) : (current + direction + ranges.length) % ranges.length;
    const [from, to] = ranges[next]!;
    editor.dispatch({
      selection: { anchor: from, head: to },
      effects: EditorView.scrollIntoView(from, { y: "center" }),
    });
    setStepped({ ranges, index: next });
  };

  return (
    <div className="relative h-full min-h-0">
      <div ref={host} className={cn("h-full min-h-0", className)} data-selectable data-testid="code-view" data-language={language} />
      {re && <MatchBar count={ranges.length} current={current} onStep={step} className="absolute top-1.5 right-4 z-10" />}
    </div>
  );
}
