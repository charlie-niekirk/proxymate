import { HighlightStyle } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

/**
 * CodeMirror chrome and syntax colours, all from the app's tokens
 * (`styles/tokens.css`), so dark and light both follow `[data-theme]`.
 */
export const codeTheme = EditorView.theme({
  "&": {
    height: "100%",
    fontSize: "11.5px",
    color: "var(--foreground)",
    backgroundColor: "var(--background)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.5", overflow: "auto" },
  ".cm-content": { padding: "4px 0", caretColor: "var(--primary)" },
  ".cm-line": { padding: "0 8px" },
  ".cm-gutters": {
    backgroundColor: "var(--code-gutter)",
    color: "var(--code-gutter-fg)",
    borderRight: "1px solid var(--border)",
  },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 6px 0 10px", minWidth: "32px" },
  ".cm-foldGutter .cm-gutterElement": { padding: "0 3px", cursor: "pointer" },
  ".cm-activeLine": { backgroundColor: "var(--code-active-line)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--code-active-line)", color: "var(--foreground)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--code-selection) !important",
  },
  ".cm-cursor": { borderLeftColor: "var(--primary)" },
  ".cm-searchMatch": { backgroundColor: "var(--code-match)", borderRadius: "2px" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--code-match-current)" },
  ".cm-selectionMatch": { backgroundColor: "var(--code-match)" },
  // M17: the filter's body terms.
  ".cm-pm-filter-match": { backgroundColor: "var(--code-match)", borderRadius: "2px", boxShadow: "inset 0 -1px 0 var(--code-match-current)" },
  ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": {
    backgroundColor: "var(--code-match)",
    outline: "1px solid var(--code-match-current)",
  },
  // Only mark brackets while the reader is in the body, not at the initial cursor.
  "&:not(.cm-focused) .cm-matchingBracket, &:not(.cm-focused) .cm-nonmatchingBracket": {
    backgroundColor: "transparent",
    outline: "none",
  },
  ".cm-foldPlaceholder": {
    backgroundColor: "var(--muted)",
    border: "1px solid var(--border)",
    color: "var(--muted-foreground)",
    padding: "0 4px",
    margin: "0 2px",
  },
  ".cm-panels": { backgroundColor: "var(--pane-header)", color: "var(--foreground)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--border)" },
  ".cm-panel.cm-search": { fontFamily: "var(--font-sans)", fontSize: "11px", padding: "4px 8px" },
  ".cm-panel.cm-search input, .cm-panel.cm-search button": {
    fontSize: "11px",
    borderRadius: "4px",
    border: "1px solid var(--input)",
    backgroundColor: "var(--background)",
    color: "var(--foreground)",
    padding: "1px 6px",
  },
  ".cm-panel.cm-search button": { backgroundImage: "none", backgroundColor: "var(--secondary)" },
  ".cm-panel.cm-search label": { fontSize: "11px" },
  ".cm-tooltip": { backgroundColor: "var(--popover)", border: "1px solid var(--border)" },
});

export const codeHighlight = HighlightStyle.define([
  { tag: [t.propertyName, t.definition(t.propertyName)], color: "var(--syntax-key)" },
  { tag: [t.string, t.special(t.string), t.attributeValue, t.regexp], color: "var(--syntax-string)" },
  { tag: [t.number, t.integer, t.float], color: "var(--syntax-number)" },
  { tag: [t.bool, t.null, t.atom, t.unit], color: "var(--syntax-literal)" },
  { tag: [t.keyword, t.controlKeyword, t.operatorKeyword, t.definitionKeyword, t.moduleKeyword], color: "var(--syntax-keyword)" },
  { tag: [t.tagName, t.angleBracket], color: "var(--syntax-tag)" },
  { tag: t.attributeName, color: "var(--syntax-attr)" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "var(--syntax-comment)", fontStyle: "italic" },
  { tag: [t.special(t.variableName), t.self, t.labelName], color: "var(--syntax-variable)" },
  { tag: [t.typeName, t.className, t.namespace], color: "var(--syntax-type)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--syntax-type)" },
  { tag: [t.punctuation, t.separator, t.bracket, t.squareBracket, t.brace, t.paren, t.operator], color: "var(--syntax-punct)" },
  { tag: [t.meta, t.documentMeta, t.processingInstruction, t.heading], color: "var(--syntax-meta)" },
  { tag: t.invalid, color: "var(--destructive)" },
  { tag: t.strong, fontWeight: "bold" },
  { tag: t.emphasis, fontStyle: "italic" },
]);
