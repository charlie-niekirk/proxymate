//! `proxymate-native`: Proxymate on gpui and gpui-kit (the Native track in
//! `docs/PLAN.md`). The binary modes (`--sysproxy-guardian`, `mcp-stdio`)
//! run first, before anything of gpui starts, exactly as in the Tauri app.
//! Then one copy per data dir: a second launch brings the first forward.

use pm_app::single_instance::{self, Instance};
use proxymate_native::app;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = pm_app::run_mode(&args) {
        std::process::exit(code);
    }
    let data_dir = app::data_dir();
    let lock = match data_dir.as_deref().map(single_instance::acquire) {
        Some(Ok(Instance::Secondary)) => {
            eprintln!("Proxymate is already running: brought it forward");
            return;
        }
        Some(Ok(Instance::Primary(lock))) => Some(lock),
        Some(Err(err)) => {
            eprintln!("can't check for a running copy: {err}");
            None
        }
        None => None,
    };
    app::run(data_dir, lock);
}
