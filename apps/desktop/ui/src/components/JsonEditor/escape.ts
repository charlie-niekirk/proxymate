/**
 * Radix dialogs close on Escape before the JSON editor's inline input sees
 * it (they listen on the document, capturing). A dialog holding the editor
 * passes this as its `onEscapeKeyDown`, so Escape cancels the edit instead.
 */
export function keepEscapeInEditor(e: KeyboardEvent) {
  const target = e.target instanceof HTMLElement ? e.target : null;
  if (target?.closest("[data-json-editor] input")) e.preventDefault();
}
