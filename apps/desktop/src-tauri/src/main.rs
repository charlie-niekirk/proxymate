// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--sysproxy-guardian") => std::process::exit(proxymate_desktop::run_guardian(&args[1..])),
        Some("mcp-stdio") => std::process::exit(proxymate_desktop::run_mcp_stdio(&args[1..])),
        _ => {}
    }
    proxymate_desktop::run();
}
