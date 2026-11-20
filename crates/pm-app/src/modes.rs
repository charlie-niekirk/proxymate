//! The binary modes the app executable doubles as. Both apps dispatch them
//! first thing in `main`, before any UI starts:
//!
//! - `--sysproxy-guardian <pid> --data-dir <dir>` ([`run_guardian`])
//! - `mcp-stdio [--port N]` ([`run_mcp_stdio`])

use std::path::PathBuf;

use pm_core::Engine;
use pm_model::MCP_DEFAULT_PORT;

/// Runs the mode `args` (the arguments after the program name) names and
/// returns its exit code, or `None` when they name none and the app should
/// start.
pub fn run_mode(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str) {
        Some(pm_platform::sysproxy::GUARDIAN_FLAG) => Some(run_guardian(&args[1..])),
        Some("mcp-stdio") => Some(run_mcp_stdio(&args[1..])),
        _ => None,
    }
}

/// `proxymate --sysproxy-guardian <pid> --data-dir <dir>`: waits for the app
/// to exit, then restores the system proxy if the app left it pointing at
/// itself (a crash or SIGKILL). Returns the process exit code.
pub fn run_guardian(args: &[String]) -> i32 {
    let pid = args.first().and_then(|p| p.parse::<u32>().ok());
    let dir = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let (Some(pid), Some(dir)) = (pid, dir) else {
        eprintln!("usage: proxymate --sysproxy-guardian <pid> --data-dir <dir>");
        return 2;
    };
    match Engine::run_system_proxy_guardian(&dir, pid) {
        Ok(services) if services.is_empty() => 0,
        Ok(services) => {
            eprintln!(
                "proxymate guardian: restored the system proxy of {}",
                services.join(", ")
            );
            0
        }
        Err(err) => {
            eprintln!("proxymate guardian: {err}");
            1
        }
    }
}

/// `proxymate mcp-stdio [--port N]`: relays a stdio MCP client to the
/// running app's endpoint, starting the app first if nothing listens.
pub fn run_mcp_stdio(args: &[String]) -> i32 {
    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(MCP_DEFAULT_PORT);
    let Some(data_dir) = crate::data_dir() else {
        eprintln!("proxymate mcp-stdio: can't find the app's data directory");
        return 1;
    };
    let config = pm_mcp::bridge::BridgeConfig {
        addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port)),
        data_dir,
        launch: Some(Box::new(|| {
            pm_platform::process::launch_app(crate::BUNDLE_ID).map_err(std::io::Error::other)
        })),
    };
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(pm_mcp::bridge::run(config)) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("proxymate mcp-stdio: {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run_mode;

    #[test]
    fn plain_launches_name_no_mode() {
        assert_eq!(run_mode(&[]), None);
        assert_eq!(run_mode(&["--some-flag".into()]), None);
    }

    #[test]
    fn a_guardian_without_its_arguments_is_a_usage_error() {
        let args = [pm_platform::sysproxy::GUARDIAN_FLAG.to_string()];
        assert_eq!(run_mode(&args), Some(2));
    }
}
