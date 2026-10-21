//! `proxymate-cli`: headless entry point. In M0 only `bench` exists; `serve`,
//! `mcp-stdio` and `gen-flows` arrive with the proxy (M1) and MCP (M7).

use std::process::ExitCode;
use std::time::Instant;

use pm_core::{Engine, synth};
use pm_model::{ContentGroup, FilterSpec, ProtocolFilter, SortDir, SortKey, SortSpec};

const USAGE: &str =
    "usage: proxymate-cli bench [FLOWS]   seed FLOWS synthetic flows (default 100000) and time refilters";

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
