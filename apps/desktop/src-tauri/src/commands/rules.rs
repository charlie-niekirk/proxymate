//! Rules and breakpoints (M5).

use pm_model::{
    BreakpointId, BreakpointInfo, BreakpointResolution, FlowId, IpcError, PresetKind, Rule, RuleBook, RuleGroup,
    RuleGroupId, RuleGroupPatch, RuleId, RuleOrigin, RuleSettings, RuleTestResult, StoreId,
};
use tauri::ipc::Channel;

use super::flows::RawBytes;
use super::{EngineState, blocking, ipc};

/// Every rule, in evaluation order.
#[tauri::command]
#[specta::specta]
pub fn list_rules(engine: EngineState<'_>) -> Vec<Rule> {
    engine.rules()
}

/// Every rule group, in order.
#[tauri::command]
#[specta::specta]
pub fn list_rule_groups(engine: EngineState<'_>) -> Vec<RuleGroup> {
    engine.rule_groups()
}

/// Saves a new, enabled group after the others.
#[tauri::command]
#[specta::specta]
pub fn create_rule_group(engine: EngineState<'_>, name: String) -> Result<RuleGroup, IpcError> {
    engine.create_rule_group(&name, true, RuleOrigin::User).map_err(ipc)
}

/// Renames a group or turns all of its rules on or off together.
#[tauri::command]
#[specta::specta]
pub fn update_rule_group(
    engine: EngineState<'_>,
    id: RuleGroupId,
    patch: RuleGroupPatch,
) -> Result<RuleGroup, IpcError> {
    engine.patch_rule_group(&id, patch).map_err(ipc)
}

/// Deletes a group, and its rules with `delete_rules` (else they stay,
/// ungrouped). Returns how many rules it held.
#[tauri::command]
#[specta::specta]
pub fn delete_rule_group(engine: EngineState<'_>, id: RuleGroupId, delete_rules: bool) -> Result<u32, IpcError> {
    engine.delete_rule_group(&id, delete_rules).map_err(ipc)
}

/// Puts the listed groups first, in that order.
#[tauri::command]
#[specta::specta]
pub fn reorder_rule_groups(engine: EngineState<'_>, ids: Vec<RuleGroupId>) -> Result<Vec<RuleGroup>, IpcError> {
    engine.reorder_rule_groups(&ids).map_err(ipc)
}

/// Moves rules to the end of a group, or out of any with `null`.
#[tauri::command]
#[specta::specta]
pub fn move_rules_to_group(
    engine: EngineState<'_>,
    ids: Vec<RuleId>,
    group: Option<RuleGroupId>,
) -> Result<Vec<Rule>, IpcError> {
    engine.move_rules_to_group(&ids, group.as_deref()).map_err(ipc)
}

/// Saves a new rule at the end of its group, or of the ungrouped rules (a
/// fresh id is assigned).
#[tauri::command]
#[specta::specta]
pub fn create_rule(engine: EngineState<'_>, rule: Rule) -> Result<Rule, IpcError> {
    engine.create_rule(rule).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn update_rule(engine: EngineState<'_>, rule: Rule) -> Result<Rule, IpcError> {
    engine.update_rule(rule).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn set_rule_enabled(engine: EngineState<'_>, id: RuleId, enabled: bool) -> Result<Rule, IpcError> {
    engine.set_rule_enabled(&id, enabled).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn delete_rule(engine: EngineState<'_>, id: RuleId) -> Result<(), IpcError> {
    engine.delete_rule(&id).map_err(ipc)
}

/// Deletes every agent-created rule; returns how many.
#[tauri::command]
#[specta::specta]
pub fn delete_agent_rules(engine: EngineState<'_>) -> Result<u32, IpcError> {
    engine.delete_agent_rules().map_err(ipc)
}

/// Puts the listed rules first, in that order.
#[tauri::command]
#[specta::specta]
pub fn reorder_rules(engine: EngineState<'_>, ids: Vec<RuleId>) -> Result<Vec<Rule>, IpcError> {
    engine.reorder_rules(&ids).map_err(ipc)
}

/// Compiles a rule without saving it; the error says what's wrong.
#[tauri::command]
#[specta::specta]
pub fn validate_rule(engine: EngineState<'_>, rule: Rule) -> Result<(), IpcError> {
    engine.validate_rule(&rule).map_err(ipc)
}

/// An unsaved rule pre-filled from a flow (in any store) by a preset.
#[tauri::command]
#[specta::specta]
pub async fn draft_rule(
    engine: EngineState<'_>,
    store: StoreId,
    flow: FlowId,
    preset: PresetKind,
) -> Result<Rule, IpcError> {
    blocking(&engine, move |engine| engine.draft_rule_in(store, flow, preset)).await
}

/// Dry run of a rule against a recorded flow; the bodies shown are cut at
/// `body_limit` bytes (default 256 KB).
#[tauri::command]
#[specta::specta]
pub async fn test_rule(
    engine: EngineState<'_>,
    store: StoreId,
    rule: Rule,
    flow: FlowId,
    body_limit: Option<u32>,
) -> Result<RuleTestResult, IpcError> {
    engine
        .test_rule_in(store, &rule, flow, body_limit.map(|n| n as usize))
        .await
        .map_err(ipc)
}

/// Flows of a store (newest first) a rule's matcher matches.
#[tauri::command]
#[specta::specta]
pub async fn flows_matching_rule(
    engine: EngineState<'_>,
    store: StoreId,
    rule: Rule,
    limit: u32,
) -> Result<Vec<FlowId>, IpcError> {
    blocking(&engine, move |engine| {
        engine.flows_matching_rule(store, &rule, limit as usize)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn get_rule_settings(engine: EngineState<'_>) -> RuleSettings {
    engine.rule_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_rule_settings(engine: EngineState<'_>, settings: RuleSettings) -> Result<RuleSettings, IpcError> {
    engine.set_rule_settings(settings).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn list_breakpoints(engine: EngineState<'_>) -> Vec<BreakpointInfo> {
    engine.breakpoints()
}

/// Streams the breakpoint queue whenever it changes.
#[tauri::command]
#[specta::specta]
pub fn subscribe_breakpoints(engine: EngineState<'_>, on_queue: Channel<Vec<BreakpointInfo>>) {
    engine.subscribe_breakpoints(Box::new(move |queue| on_queue.send(queue).is_ok()));
}

/// A paused message's (decoded) body as a raw `ArrayBuffer`.
#[tauri::command]
#[specta::specta]
pub async fn get_breakpoint_body(engine: EngineState<'_>, id: BreakpointId) -> Result<RawBytes, IpcError> {
    blocking(&engine, move |engine| {
        engine
            .breakpoint_body(id)
            .map(|bytes| RawBytes(tauri::ipc::Response::new(bytes)))
    })
    .await
}

/// Continues, aborts or answers a paused flow. The first resolution wins.
#[tauri::command]
#[specta::specta]
pub fn resolve_breakpoint(
    engine: EngineState<'_>,
    id: BreakpointId,
    resolution: BreakpointResolution,
) -> Result<(), IpcError> {
    engine.resolve_breakpoint(id, resolution).map_err(ipc)
}

/// Streams the rules and groups whenever anyone (the UI or an agent)
/// changes them.
#[tauri::command]
#[specta::specta]
pub fn subscribe_rules(engine: EngineState<'_>, on_rules: Channel<RuleBook>) {
    engine.subscribe_rules(Box::new(move |book| on_rules.send(book).is_ok()));
}
