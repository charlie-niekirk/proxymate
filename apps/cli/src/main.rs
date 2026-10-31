//! `proxymate-cli`: headless entry point. `serve` runs the proxy and logs
//! flows (and with `--mcp`, the MCP server); `mcp-stdio` bridges a stdio
//! MCP client to a running app or `serve --mcp`; `bench` times the store,
//! or with `--rps` soaks a proxy with a steady mix of traffic.

mod soak;

use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::time::Instant;

use pm_core::{Engine, EngineConfig, ProxyConfig, synth};
use pm_model::{
    ContentGroup, FilterSpec, FlowState, FlowSummary, MCP_DEFAULT_PORT, ProtocolFilter, SortDir, SortKey, SortSpec,
    TlsStatus,
};

const USAGE: &str = "usage:
  proxymate-cli serve [PORT] [--mcp[=MCP_PORT]]
                                run the proxy on 127.0.0.1:PORT (default 9090, falls back up to 9099) and log flows;
                                with --mcp also serve MCP on 127.0.0.1:MCP_PORT/mcp (default 9095)
                                env: PROXYMATE_DATA_DIR (CA, settings, MCP token; default: a temp dir),
                                     PROXYMATE_EXTRA_UPSTREAM_CA (PEM of extra upstream roots)
  proxymate-cli mcp-stdio [--port MCP_PORT] [--no-launch]
                                relay a stdio MCP client to the MCP endpoint, starting the Proxymate app if
                                nothing listens; the token is read from PROXYMATE_DATA_DIR (default: the app's)
  proxymate-cli bench [FLOWS]   seed FLOWS synthetic flows (default 100000) and time refilters
  proxymate-cli bench --rps N [--duration SECS] [--interval SECS] [--max-flows N] [--proxy HOST:PORT --pid PID]
                                soak: N requests a second of mixed bodies through a proxy to a local test upstream,
                                printing CSV (rate, latency, failures, RSS, flows, disk) every interval (default 30 s);
                                its own engine unless --proxy names one (e.g. the app, whose pid RSS is sampled)";

/// The desktop app's bundle id, for starting it from `mcp-stdio`.
const APP_BUNDLE_ID: &str = "dev.proxymate.desktop";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mcp-stdio") => mcp_stdio(&args[1..]),
        Some("bench") if args.iter().any(|a| a == "--rps") => match soak_config(&args[1..]) {
            Some(config) => {
                soak::run(config);
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("{USAGE}");
                ExitCode::FAILURE
            }
        },
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
            let mut ports = ProxyConfig::default().ports;
            let mut mcp = None;
            for arg in &args[1..] {
                match arg.as_str() {
                    "--mcp" => mcp = Some(MCP_DEFAULT_PORT),
                    a if a.starts_with("--mcp=") => match a["--mcp=".len()..].parse() {
                        Ok(port) => mcp = Some(port),
                        Err(_) => {
                            eprintln!("{USAGE}");
                            return ExitCode::FAILURE;
                        }
                    },
                    a => match a.parse::<u16>() {
                        Ok(port) => ports = port..=port,
                        Err(_) => {
                            eprintln!("{USAGE}");
                            return ExitCode::FAILURE;
                        }
                    },
                }
            }
            serve(
                ProxyConfig {
                    ports,
                    ..Default::default()
                },
                mcp,
            )
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// `--rps N [--duration S] [--interval S] [--max-flows N] [--proxy A] [--pid P]`.
fn soak_config(args: &[String]) -> Option<soak::SoakConfig> {
    let mut config = soak::SoakConfig {
        rps: 500,
        duration: std::time::Duration::from_secs(60),
        interval: std::time::Duration::from_secs(30),
        proxy: None,
        pid: None,
        max_flows: None,
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next()?;
        match flag.as_str() {
            "--rps" => config.rps = value.parse().ok().filter(|&n| n > 0)?,
            "--duration" => config.duration = std::time::Duration::from_secs(value.parse().ok()?),
            "--interval" => config.interval = std::time::Duration::from_secs(value.parse().ok().filter(|&n| n > 0)?),
            "--max-flows" => config.max_flows = Some(value.parse().ok()?),
            "--proxy" => config.proxy = Some(value.parse().ok()?),
            "--pid" => config.pid = Some(value.parse().ok()?),
            _ => return None,
        }
    }
    Some(config)
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
                ..Default::default()
            },
        )
        .expect("view exists");
    println!(
        "sort by url             {:>7} rows  {:>6.2} ms",
        info.len,
        info.elapsed_us as f64 / 1e3
    );
}

fn serve(config: ProxyConfig, mcp: Option<u16>) -> ExitCode {
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
            ..Default::default()
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
        let _mcp = match mcp {
            Some(port) => match tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => match pm_mcp::serve(engine.clone(), listener).await {
                    Ok(server) => {
                        engine.set_mcp_serving(Some(server.url()), None);
                        println!("MCP: {}", server.url());
                        println!(
                            "  claude mcp add --transport http proxymate {} --header \"Authorization: Bearer {}\"",
                            server.url(),
                            engine.mcp_token()
                        );
                        Some(server)
                    }
                    Err(err) => {
                        eprintln!("MCP: {err}");
                        return ExitCode::FAILURE;
                    }
                },
                Err(err) => {
                    eprintln!("MCP: can't listen on 127.0.0.1:{port}: {err}");
                    return ExitCode::FAILURE;
                }
            },
            None => None,
        };

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

fn mcp_stdio(args: &[String]) -> ExitCode {
    let mut port = MCP_DEFAULT_PORT;
    let mut launch = true;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--port" => match rest.next().and_then(|p| p.parse().ok()) {
                Some(p) => port = p,
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::FAILURE;
                }
            },
            "--no-launch" => launch = false,
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(data_dir) = std::env::var_os("PROXYMATE_DATA_DIR")
        .map(Into::into)
        .or_else(pm_platform::fs::app_data_dir)
    else {
        eprintln!("proxymate-cli mcp-stdio: no data directory; set PROXYMATE_DATA_DIR");
        return ExitCode::FAILURE;
    };
    let config = pm_mcp::bridge::BridgeConfig {
        addr: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        data_dir,
        launch: launch.then(|| {
            Box::new(|| pm_platform::process::launch_app(APP_BUNDLE_ID).map_err(std::io::Error::other))
                as Box<dyn Fn() -> std::io::Result<()> + Send>
        }),
    };
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(pm_mcp::bridge::run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("proxymate-cli mcp-stdio: {err}");
            ExitCode::FAILURE
        }
    }
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
