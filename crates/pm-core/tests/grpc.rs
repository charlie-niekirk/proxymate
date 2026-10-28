//! gRPC end to end (M16): client → CONNECT → MITM (h2) → the testkit's
//! tonic upstream. Trailers are recorded, the status reaches the summary,
//! and `Engine::decode_body_in` decodes the messages with and without the
//! imported schema, live streams included. gRPC-web carries its trailers
//! in the body.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use pm_core::{Engine, EngineConfig, ProtoImport, ProxyConfig};
use pm_model::{
    BodyPart, ContentKind, DecodedBody, DecodedFormat, FilterSpec, FlowDetail, FlowId, FlowSelection, FlowState,
    FlowSummary, LIVE_STORE, Matcher, Pattern, ProtoMapping, SortSpec,
};
use pm_testkit::GrpcUpstream;
use pm_testkit::client::eventually;
use pm_testkit::grpc::{self, SHOP_PROTO, TOTAL_CENTS, shop};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

const WAIT: Duration = Duration::from_secs(10);
const GET_ORDER: &str = "/shop.v1.Orders/GetOrder";
const LIST_ORDERS: &str = "/shop.v1.Orders/ListOrders";

struct Proxy {
    engine: Arc<Engine>,
    addr: SocketAddr,
    dir: std::path::PathBuf,
}

impl Proxy {
    async fn start(upstream: &GrpcUpstream) -> Proxy {
        let dir = std::env::temp_dir()
            .join("proxymate-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(dir.clone()),
            extra_upstream_roots: vec![upstream.ca_der()],
            ..Default::default()
        });
        let addr = engine
            .start_proxy(ProxyConfig {
                ports: 0..=0,
                ..Default::default()
            })
            .await
            .expect("proxy starts");
        Proxy { engine, addr, dir }
    }

    fn ca(&self) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(self.engine.ca_info().cert_path).expect("CA PEM")
    }

    async fn finished(&self, id: FlowId) -> (FlowSummary, FlowDetail) {
        let summary = eventually(WAIT, || {
            self.engine.flush();
            self.engine
                .get_flow(id)
                .filter(|f| f.state != FlowState::Active && f.grpc_status.is_some())
        })
        .await;
        let detail = self.engine.flow_detail(id).unwrap().expect("detail");
        (summary, detail)
    }

    fn decode(&self, id: FlowId, part: BodyPart) -> DecodedBody {
        self.engine.decode_body_in(LIVE_STORE, id, part).expect("decodes")
    }

    fn import_shop(&self) {
        self.engine
            .import_proto_schema(
                "shop",
                ProtoImport::Sources(vec![("shop.proto".into(), SHOP_PROTO.into())]),
            )
            .expect("shop.proto imports");
    }
}

fn json(message: &pm_model::DecodedMessage) -> serde_json::Value {
    serde_json::from_str(&message.json).expect("valid JSON")
}

#[tokio::test]
async fn unary_call_records_trailers_and_decodes_with_and_without_a_schema() {
    let upstream = GrpcUpstream::start().await;
    let proxy = Proxy::start(&upstream).await;
    let request = shop::GetOrderRequest { id: "o1".into() };
    let response = grpc::call(
        proxy.addr,
        upstream.port(),
        &proxy.ca(),
        GET_ORDER,
        &request,
        Some("gzip"),
    )
    .await;
    assert_eq!((response.status, response.grpc_status()), (200, Some(0)));
    assert_eq!(response.headers.get("grpc-encoding").unwrap(), "gzip");

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(
        (flow.state, flow.content_kind),
        (FlowState::Completed, ContentKind::Grpc)
    );
    assert_eq!(flow.grpc_status, Some(0));
    let trailers = detail.trailers.expect("trailers recorded");
    assert!(
        trailers.iter().any(|h| h.name == "grpc-status" && h.value == "0"),
        "{trailers:?}"
    );

    // Schema-less: field numbers and wire types.
    let decoded = proxy.decode(0, BodyPart::Response);
    assert_eq!(decoded.format, DecodedFormat::Grpc);
    assert_eq!(decoded.status.as_ref().map(|s| s.name.as_str()), Some("OK"));
    assert_eq!(decoded.messages.len(), 1);
    let message = &decoded.messages[0];
    assert!(message.compressed && message.schema_less);
    let value = json(message);
    assert_eq!(value["1"], "o1");
    assert_eq!(value["2"].as_u64(), Some(TOTAL_CENTS as u64));
    assert_eq!(value["3"][0]["1"], "tea-earl-grey");
    assert!(
        message
            .badges
            .iter()
            .any(|b| b.pointer == "/3" && b.label == "repeated")
    );
    let request = proxy.decode(0, BodyPart::Request);
    assert_eq!(json(&request.messages[0])["1"], "o1");
    assert_eq!(request.status, None);

    // With the schema, the path finds both types.
    proxy.import_shop();
    let decoded = proxy.decode(0, BodyPart::Response);
    assert_eq!(decoded.message_type.as_deref(), Some("shop.v1.Order"));
    let value = json(&decoded.messages[0]);
    assert!(!decoded.messages[0].schema_less);
    assert_eq!(value["totalCents"], TOTAL_CENTS.to_string());
    assert_eq!(value["state"], "SHIPPED");
    assert_eq!(value["placedAt"], "2023-11-14T22:13:20Z");
    assert_eq!(value["items"][1]["sku"], "mug");
    let request = proxy.decode(0, BodyPart::Request);
    assert_eq!(request.message_type.as_deref(), Some("shop.v1.GetOrderRequest"));
    assert_eq!(json(&request.messages[0])["id"], "o1");

    // The schema survives a restart of the engine (app.sqlite).
    let again = Engine::with_config(EngineConfig {
        data_dir: Some(proxy.dir.clone()),
        ..Default::default()
    });
    let schemas = again.proto_schemas();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].services, ["shop.v1.Orders"]);
    assert!(again.proto_message_types().contains(&"shop.v1.LineItem".to_owned()));
}

#[tokio::test]
async fn an_error_status_is_decoded_with_its_details() {
    let upstream = GrpcUpstream::start().await;
    let proxy = Proxy::start(&upstream).await;
    let request = shop::GetOrderRequest { id: "missing".into() };
    let response = grpc::call(proxy.addr, upstream.port(), &proxy.ca(), GET_ORDER, &request, None).await;
    assert_eq!(response.grpc_status(), Some(5));

    let (flow, _) = proxy.finished(0).await;
    assert_eq!(flow.grpc_status, Some(5));
    let status = proxy.decode(0, BodyPart::Response).status.expect("status");
    assert_eq!((status.code, status.name.as_str()), (5, "NOT_FOUND"));
    assert_eq!(status.message.as_deref(), Some("no order “missing”"));
    let details: serde_json::Value = serde_json::from_str(&status.details.expect("details")).unwrap();
    assert_eq!(details["code"], 5);
    assert_eq!(
        details["details"][0]["@type"],
        "type.googleapis.com/google.rpc.ResourceInfo"
    );
    assert_eq!(details["details"][0]["value"]["2"], "missing");

    // The DSL finds failed calls.
    let filter = |text: &str| FilterSpec {
        text: text.into(),
        ..Default::default()
    };
    assert_eq!(proxy.engine.create_view(filter("grpc:!0"), SortSpec::default()).len, 1);
    assert_eq!(proxy.engine.create_view(filter("grpc:0"), SortSpec::default()).len, 0);
}

#[tokio::test]
async fn a_server_stream_decodes_as_it_arrives() {
    let upstream = GrpcUpstream::start().await;
    let proxy = Proxy::start(&upstream).await;
    proxy.import_shop();
    let request = shop::ListOrdersRequest {
        count: 4,
        interval_ms: 400,
    };
    let (addr, port, ca) = (proxy.addr, upstream.port(), proxy.ca());
    let call = tokio::spawn(async move { grpc::call(addr, port, &ca, LIST_ORDERS, &request, Some("zstd")).await });

    // Part of the stream decodes while the flow is still active.
    let partial = eventually(WAIT, || {
        proxy.engine.flush();
        let flow = proxy.engine.get_flow(0)?;
        let decoded = proxy.engine.decode_body_in(LIVE_STORE, 0, BodyPart::Response).ok()?;
        (flow.state == FlowState::Active && !decoded.messages.is_empty()).then_some(decoded)
    })
    .await;
    assert!(partial.messages.len() < 4, "{} messages", partial.messages.len());
    assert_eq!(partial.status, None);

    let response = call.await.unwrap();
    assert_eq!(response.grpc_status(), Some(0));
    let (flow, _) = proxy.finished(0).await;
    assert_eq!(flow.grpc_status, Some(0));
    let decoded = proxy.decode(0, BodyPart::Response);
    assert_eq!(decoded.messages.len(), 4);
    assert!(decoded.messages.iter().all(|m| m.compressed && !m.schema_less));
    let ids: Vec<String> = decoded
        .messages
        .iter()
        .map(|m| json(m)["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["o1", "o2", "o3", "o4"]);
    assert_eq!(decoded.pending_bytes, 0);
}

#[tokio::test]
async fn grpc_web_trailers_come_from_the_body() {
    let upstream = GrpcUpstream::start().await;
    let proxy = Proxy::start(&upstream).await;
    let request = shop::GetOrderRequest { id: "web".into() };
    for (id, text) in [(0, false), (1, true)] {
        let response = grpc::web_call(proxy.addr, upstream.port(), &proxy.ca(), GET_ORDER, &request, text).await;
        assert_eq!(response.status, 200);
        let (flow, detail) = proxy.finished(id).await;
        assert_eq!((flow.content_kind, flow.grpc_status), (ContentKind::Grpc, Some(0)));
        let trailers = detail.trailers.expect("trailer frame kept as trailers");
        assert!(trailers.iter().any(|h| h.name == "grpc-status"), "{trailers:?}");
        let decoded = proxy.decode(id, BodyPart::Response);
        assert_eq!(decoded.format, DecodedFormat::GrpcWeb);
        assert_eq!(decoded.messages.len(), 1, "text: {text}");
        assert_eq!(json(&decoded.messages[0])["1"], "web");
        assert_eq!(decoded.status.map(|s| s.code), Some(0));
    }
}

#[tokio::test]
async fn trailers_and_status_survive_a_saved_session() {
    let upstream = GrpcUpstream::start().await;
    let proxy = Proxy::start(&upstream).await;
    // A call with trailers, and an error, which tonic sends trailers-only
    // (the status in the response headers).
    for (id, order) in [(0, "o1"), (1, "missing")] {
        let request = shop::GetOrderRequest { id: order.into() };
        grpc::call(proxy.addr, upstream.port(), &proxy.ca(), GET_ORDER, &request, None).await;
        proxy.finished(id).await;
    }

    let path = proxy.dir.join("grpc.proxymate");
    let (tx, rx) = std::sync::mpsc::channel();
    proxy
        .engine
        .save_session(
            LIVE_STORE,
            &FlowSelection::All,
            path.clone(),
            Box::new(move |p| {
                let _ = tx.send(p);
            }),
        )
        .unwrap();
    while !rx.recv_timeout(WAIT).expect("save progress").finished {}
    let store = proxy.engine.open_document(&path).unwrap().store;
    assert_eq!(proxy.engine.get_flow_in(store, 0).unwrap().grpc_status, Some(0));
    let detail = proxy.engine.flow_detail_in(store, 0).unwrap().unwrap();
    assert!(detail.trailers.unwrap().iter().any(|h| h.name == "grpc-status"));
    assert_eq!(proxy.engine.get_flow_in(store, 1).unwrap().grpc_status, Some(5));
    let detail = proxy.engine.flow_detail_in(store, 1).unwrap().unwrap();
    assert_eq!(detail.trailers, None);
    let decoded = proxy.engine.decode_body_in(store, 1, BodyPart::Response).unwrap();
    assert_eq!(decoded.status.map(|s| s.name), Some("NOT_FOUND".into()));
}

#[tokio::test]
async fn decode_as_mappings_name_a_type() {
    let upstream = GrpcUpstream::start().await;
    let proxy = Proxy::start(&upstream).await;
    proxy.import_shop();
    let request = shop::GetOrderRequest { id: "o1".into() };
    grpc::call(proxy.addr, upstream.port(), &proxy.ca(), GET_ORDER, &request, None).await;
    proxy.finished(0).await;

    // A mapping beats the path: here the request read as a LineItem.
    let mapping = ProtoMapping {
        id: "m1".into(),
        matcher: Matcher {
            path: Some(Pattern::exact(GET_ORDER)),
            ..Default::default()
        },
        part: Some(BodyPart::Request),
        message_type: "shop.v1.LineItem".into(),
    };
    proxy.engine.set_proto_mappings(vec![mapping.clone()]).unwrap();
    assert_eq!(proxy.engine.proto_mappings(), std::slice::from_ref(&mapping));
    let decoded = proxy.decode(0, BodyPart::Request);
    assert_eq!(decoded.message_type.as_deref(), Some("shop.v1.LineItem"));
    assert_eq!(json(&decoded.messages[0])["sku"], "o1");
    // The response still follows the path.
    let decoded = proxy.decode(0, BodyPart::Response);
    assert_eq!(decoded.message_type.as_deref(), Some("shop.v1.Order"));

    let unknown = ProtoMapping {
        message_type: "nope.Missing".into(),
        ..mapping
    };
    assert!(proxy.engine.set_proto_mappings(vec![unknown]).is_err());
}
