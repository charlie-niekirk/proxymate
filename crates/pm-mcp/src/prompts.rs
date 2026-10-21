//! MCP prompts: ready-made starts for the two jobs agents do most here,
//! finding out why requests fail and mocking an endpoint.

use rmcp::ErrorData;
use rmcp::model::{GetPromptResult, JsonObject, Prompt, PromptArgument, PromptMessage, Role};

pub(crate) fn list() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "investigate_failures",
            Some("Find failing requests in the recorded traffic and explain why they fail."),
            Some(vec![
                PromptArgument::new("filter")
                    .with_description("Filter DSL to narrow the traffic, e.g. `host:api.example.com`.")
                    .with_required(false),
            ]),
        ),
        Prompt::new(
            "mock_endpoint",
            Some("Answer an endpoint locally (Map Local) instead of hitting the server."),
            Some(vec![
                PromptArgument::new("url")
                    .with_description("URL or pattern of the endpoint, e.g. `https://api.example.com/v1/users/*`.")
                    .with_required(true),
                PromptArgument::new("response")
                    .with_description("What the mock should return; default: what the server returned last time.")
                    .with_required(false),
            ]),
        ),
    ]
}

fn arg(args: &JsonObject, name: &str) -> Option<String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn get(name: &str, args: JsonObject) -> Result<GetPromptResult, ErrorData> {
    match name {
        "investigate_failures" => {
            let scope = arg(&args, "filter").map(|f| format!(" {f}")).unwrap_or_default();
            let text = format!(
                "Investigate failing HTTP traffic recorded by Proxymate.\n\n\
                 1. Call list_flows with filter `status:4xx,5xx{scope}`, then with `state:failed{scope}` \
                 (connection and TLS errors, which have no status).\n\
                 2. Group the failures by host, path and status, and pick the most frequent or most recent ones.\n\
                 3. For each group, call get_flow on one example (the `error` field explains failed flows, the \
                 `tls` include explains certificate problems) and get_flow_body for its response body.\n\
                 4. Where it helps, compare with a successful flow to the same endpoint \
                 (`status:2xx path:<path>`), looking at request headers, auth, and the body sent.\n\
                 5. Summarise each failure: what was requested, what came back, the likely cause, and how to fix \
                 it. Cite flow ids."
            );
            Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
                .with_description("Find and explain failing requests"))
        }
        "mock_endpoint" => {
            let url = arg(&args, "url").ok_or_else(|| ErrorData::invalid_params("mock_endpoint needs a url", None))?;
            let response = arg(&args, "response")
                .map(|r| format!("The mock should return: {r}"))
                .unwrap_or_else(|| "Unless told otherwise, start from what the server returned last time.".to_owned());
            let text = format!(
                "Mock `{url}` with Proxymate so the app gets a local answer instead of the server's.\n\n\
                 1. Call list_flows with a filter for this endpoint (e.g. `host:<host> path:<path>`) to find a \
                 recorded example, and get_flow_body to see its response.\n\
                 2. {response}\n\
                 3. Call create_map_local with `url` (or the example's `flow_id`), the status, the body and \
                 `content_type`. Use list_rules (origin: agent) to check there isn't an older mock for it, and \
                 update or delete it instead of stacking rules.\n\
                 4. Ask the user to repeat the request (or wait_for_flow on it) and confirm with get_flow that \
                 `rules.mocked` is true.\n\
                 5. Report the rule id; delete_rule (or delete_agent_rules) removes the mock."
            );
            Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
                .with_description("Mock an endpoint with Map Local"))
        }
        other => Err(ErrorData::invalid_params(format!("unknown prompt {other:?}"), None)),
    }
}
