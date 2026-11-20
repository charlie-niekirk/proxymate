// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = pm_app::run_mode(&args) {
        std::process::exit(code);
    }
    proxymate_desktop::run();
}
