//! `proxymate-cli`: headless entry point. `serve` runs the proxy and logs
//! flows; `bench` times the store. `mcp-stdio` arrives with MCP (M7).

use std::process::ExitCode;
use std::time::Instant;

use pm_core::{Engine, EngineConfig, ProxyConfig, synth};
use pm_model::{
    ContentGroup, FilterSpec, FlowState, FlowSummary, ProtocolFilter, SortDir, SortKey, SortSpec, TlsStatus,
};

const USAGE: &str = "usage:
  proxymate-cli serve [PORT]    run the proxy on 127.0.0.1:PORT (default 9090, falls back up to 9099) and log flows
                                env: PROXYMATE_DATA_DIR (CA, settings; default: a temp dir),
                                     PROXYMATE_EXTRA_UPSTREAM_CA (PEM of extra upstream roots)
  proxymate-cli bench [FLOWS]   seed FLOWS synthetic flows (default 100000) and time refilters";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bench") => {
            let flows = match args.get(1).map(|s| s.parse::<usize>()) {
                None => 100_000,
                Some(Ok(n)) => n,
                Some(Err(_)) => {
                    eprintln!("{USAGE}");
                    return ExitCode::FAILURE;
                }
            };
            bench(flows);
            ExitCode::SUCCESS
        }
        Some("serve") => {
            let ports = match args.get(1).map(|s| s.parse::<u16>()) {
                None => ProxyConfig::default().ports,
                Some(Ok(port)) => port..=port,
                Some(Err(_)) => {
                    eprintln!("{USAGE}");
                    return ExitCode::FAILURE;
                }
            };
            serve(ProxyConfig {
                ports,
                ..Default::default()
            })
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn bench(flows: usize) {
    let engine = Engine::new();
    let start = Instant::now();
    synth::seed(&engine, flows, 1);
    println!("seed {flows} flows: {:.1} ms", start.elapsed().as_secs_f64() * 1e3);

    let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;
    let cases = [
        (
            "text \"api\"",
            FilterSpec {
                text: "api".into(),
                ..Default::default()
            },
        ),
        (
            "https + json",
            FilterSpec {
                protocol: ProtocolFilter::Https,
                content: vec![ContentGroup::Json],
                ..Default::default()
            },
        ),
        (
            "no match",
            FilterSpec {
                text: "zzzz-not-there".into(),
                ..Default::default()
            },
        ),
        ("all", FilterSpec::default()),
    ];
    for (name, spec) in cases {
        let info = engine.set_filter(view, spec).expect("view exists");
        println!(
            "refilter {name:<14} {:>7} rows  {:>6.2} ms",
            info.len,
            info.elapsed_us as f64 / 1e3
        );
    }
    let info = engine
        .set_sort(
            view,
            SortSpec {
                key: SortKey::Url,
                dir: SortDir::Asc,
            },
        )
        .expect("view exists");
    println!(
        "sort by url             {:>7} rows  {:>6.2} ms",
        info.len,
        info.elapsed_us as f64 / 1e3
    );
}

fn serve(config: ProxyConfig) -> ExitCode {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let extra_upstream_roots = match std::env::var_os("PROXYMATE_EXTRA_UPSTREAM_CA") {
            Some(path) => match pm_core::tls::certs_from_pem_file(path.as_ref()) {
                Ok(certs) => certs,
                Err(err) => {
                    eprintln!("PROXYMATE_EXTRA_UPSTREAM_CA: {err}");
                    return ExitCode::FAILURE;
                }
            },
            None => Vec::new(),
        };
        let engine = Engine::with_config(EngineConfig {
            data_dir: std::env::var_os("PROXYMATE_DATA_DIR").map(Into::into),
            extra_upstream_roots,
        });
        let addr = match engine.start_proxy(config).await {
            Ok(addr) => addr,
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        };
        let ca = engine.ca_info();
        println!("listening on http://{addr}  (try: curl -x http://{addr} http://example.com/)");
        println!(
            "HTTPS CA: {}  (try: curl -x http://{addr} --cacert {} https://example.com/)",
            ca.common_name, ca.cert_path
        );

        // Log each flow once it finishes. Short flows arrive finished, as
        // appended rows; longer ones finish later and come back as changes.
        let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;
        let log = std::sync::Arc::downgrade(&engine);
        engine
            .subscribe(
                view,
                Box::new(move |delta| {
                    let Some(log) = log.upgrade() else {
                        return false;
                    };
                    let appended = delta.len - delta.appended..delta.len;
                    let rows = log.get_rows(view, appended.start, delta.appended).map(|p| p.rows);
                    for id in rows.unwrap_or_default().iter().map(|r| r.0).chain(delta.changed) {
                        if let Some(flow) = log.get_flow(id).filter(|f| f.state != FlowState::Active) {
                            println!("{}", describe(&flow));
                        }
                    }
                    true
                }),
            )
            .expect("view exists");
        let _ = tokio::signal::ctrl_c().await;
        engine.shutdown();
        ExitCode::SUCCESS
    })
}

fn describe(flow: &FlowSummary) -> String {
    let status = match (flow.state, flow.status) {
        (FlowState::Failed, _) if flow.tls == TlsStatus::LikelyPinned => "PINNED".to_owned(),
        (FlowState::Failed, _) => "FAIL".to_owned(),
        (FlowState::Tunnel, _) => "TUNNEL".to_owned(),
        (_, Some(status)) => status.to_string(),
        (_, None) => "-".to_owned(),
    };
    let query = if flow.query.is_empty() {
        String::new()
    } else {
        format!("?{}", flow.query)
    };
    format!(
        "#{:<5} {:<7} {status:<6} {:<8} {}://{}:{}{}{query}  {} B  {} ms",
        flow.id,
        flow.method,
        flow.http_version.as_str(),
        flow.scheme.as_str(),
        flow.host,
        flow.port,
        flow.path,
        flow.resp_size,
        flow.duration_ms.unwrap_or(0)
    )
}
