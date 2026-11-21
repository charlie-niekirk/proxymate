//! `PROXYMATE_DEV_ZOO=1` (`dev/zoo.ts`): one flow per body-viewer zoo
//! fixture (`crates/pm-testkit/fixtures/zoo/`), sent through the app's own
//! proxy from a pm-testkit upstream on loopback, for viewer screenshots.
//! The upstream stays up while the app runs, so Replay works on them too.

use std::time::Duration;

use gpui_kit::App;

pub fn start(cx: &mut App) {
    if !crate::dev_env::flag("PROXYMATE_DEV_ZOO") {
        return;
    }
    let engine = crate::bridge::Bridge::engine(cx);
    crate::bridge::Bridge::runtime(cx).spawn(async move {
        // The proxy binds as the engine starts; wait for its address.
        let mut proxy = None;
        for _ in 0..100 {
            let status = engine.proxy_status();
            if let Some(port) = status
                .listening
                .as_deref()
                .and_then(|a| a.rsplit(':').next()?.parse::<u16>().ok())
            {
                proxy = Some(std::net::SocketAddr::from(([127, 0, 0, 1], port)));
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let Some(proxy) = proxy else {
            return eprintln!("dev zoo: the proxy isn't listening");
        };
        let upstream = pm_testkit::Upstream::start().await;
        let sent = pm_testkit::zoo::send_all(proxy, upstream.addr()).await;
        eprintln!("dev zoo: sent {} fixtures through {proxy}", sent.len());
        // Keep the upstream for replays.
        std::future::pending::<()>().await;
        drop(upstream);
    });
}
