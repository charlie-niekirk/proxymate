//! Rules and breakpoints (M5).

use std::sync::Arc;

use pm_model::{
    BreakpointId, BreakpointInfo, BreakpointResolution, FlowId, PresetKind, Rule, RuleId, RuleSettings, RuleTestResult,
    StoreId,
};
use tauri::ipc::Channel;

use super::flows::RawBytes;
use super::{EngineState, run_blocking};

/// Every rule, in evaluation order.
#[tauri::command]
#[specta::specta]
pub fn list_rules(engine: EngineState<'_>) -> Vec<Rule> {
    engine.rules()
}

/// Saves a new rule at the end of the list (a fresh id is assigned).
#[tauri::command]
#[specta::specta]
pub fn create_rule(engine: EngineState<'_>, rule: Rule) -> Result<Rule, String> {
    engine.create_rule(rule).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn update_rule(engine: EngineState<'_>, rule: Rule) -> Result<Rule, String> {
    engine.update_rule(rule).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_rule_enabled(engine: EngineState<'_>, id: RuleId, enabled: bool) -> Result<Rule, String> {
    engine.set_rule_enabled(&id, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn delete_rule(engine: EngineState<'_>, id: RuleId) -> Result<(), String> {
    engine.delete_rule(&id).map_err(|e| e.to_string())
}

/// Deletes every agent-created rule; returns how many.
#[tauri::command]
#[specta::specta]
pub fn delete_agent_rules(engine: EngineState<'_>) -> Result<u32, String> {
    engine.delete_agent_rules().map_err(|e| e.to_string())
}

/// Puts the listed rules first, in that order.
#[tauri::command]
#[specta::specta]
pub fn reorder_rules(engine: EngineState<'_>, ids: Vec<RuleId>) -> Result<Vec<Rule>, String> {
    engine.reorder_rules(&ids).map_err(|e| e.to_string())
}

/// Compiles a rule without saving it; the error says what's wrong.
#[tauri::command]
#[specta::specta]
pub fn validate_rule(engine: EngineState<'_>, rule: Rule) -> Result<(), String> {
    engine.validate_rule(&rule).map_err(|e| e.to_string())
}

/// An unsaved rule pre-filled from a flow (in any store) by a preset.
#[tauri::command]
#[specta::specta]
pub async fn draft_rule(
    engine: EngineState<'_>,
    store: StoreId,
    flow: FlowId,
    preset: PresetKind,
) -> Result<Rule, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.draft_rule_in(store, flow, preset).map_err(|e| e.to_string())).await
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
) -> Result<RuleTestResult, String> {
    engine
        .test_rule_in(store, &rule, flow, body_limit.map(|n| n as usize))
        .await
        .map_err(|e| e.to_string())
}

/// Flows of a store (newest first) a rule's matcher matches.
#[tauri::command]
#[specta::specta]
pub async fn flows_matching_rule(
    engine: EngineState<'_>,
    store: StoreId,
    rule: Rule,
    limit: u32,
) -> Result<Vec<FlowId>, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .flows_matching_rule(store, &rule, limit as usize)
            .map_err(|e| e.to_string())
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
pub fn set_rule_settings(engine: EngineState<'_>, settings: RuleSettings) -> Result<RuleSettings, String> {
    engine.set_rule_settings(settings).map_err(|e| e.to_string())
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
pub async fn get_breakpoint_body(engine: EngineState<'_>, id: BreakpointId) -> Result<RawBytes, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .breakpoint_body(id)
            .map(|bytes| RawBytes(tauri::ipc::Response::new(bytes)))
            .map_err(|e| e.to_string())
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
) -> Result<(), String> {
    engine.resolve_breakpoint(id, resolution).map_err(|e| e.to_string())
}

/// Streams the rule list whenever anyone (the UI or an agent) changes it.
#[tauri::command]
#[specta::specta]
pub fn subscribe_rules(engine: EngineState<'_>, on_rules: Channel<Vec<Rule>>) {
    engine.subscribe_rules(Box::new(move |rules| on_rules.send(rules).is_ok()));
}
