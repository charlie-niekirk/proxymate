/** The flow table's DOM id, for "focus the table" from shortcuts and the inspector. */
export const FLOW_TABLE_ID = "flow-table";

/** Moves keyboard focus to the flow table (Escape from the inspector, the palette). */
export function focusFlowTable() {
  document.getElementById(FLOW_TABLE_ID)?.focus();
}

/** Moves keyboard focus into the inspector: its active Response tab. */
export function focusInspector() {
  const tab = document.querySelector<HTMLElement>('[data-testid="inspector"] [role="tab"][data-state="active"]');
  tab?.focus();
}
