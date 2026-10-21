//! The upstream client must connect directly even when the environment
//! names a proxy: once Proxymate is the system proxy, honouring it would
//! loop every request back into Proxymate.

use std::time::Duration;

use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_testkit::Upstream;
use pm_testkit::client;

#[test]
fn proxy_environment_variables_are_ignored() {
    // A proxy nobody listens on: any request that honoured it would fail.
    // Safety: set before the runtime or any other thread exists, and this
    // test binary has no other tests that could read the environment.
    #[allow(unsafe_code)]
    unsafe {
        for key in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
            std::env::set_var(key, "http://127.0.0.1:9");
        }
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let upstream = Upstream::start().await;
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(std::env::temp_dir().join("proxymate-tests")),
        });
        let proxy = engine
            .start_proxy(ProxyConfig {
                ports: 0..=0,
                ..Default::default()
            })
            .await
            .unwrap();
        let (status, _) = tokio::time::timeout(Duration::from_secs(10), client::get(proxy, &upstream.url("/json")))
            .await
            .unwrap();
        assert_eq!(status, 200);
    });
}
