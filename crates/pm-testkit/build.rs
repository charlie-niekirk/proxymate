//! Generates the gRPC upstream's messages and service from
//! `proto/shop.proto`, compiled by protox: no `protoc` needed.

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=proto/shop.proto");
    let fds = protox::compile(["shop.proto"], ["proto"]).map_err(std::io::Error::other)?;
    tonic_prost_build::configure().build_transport(false).compile_fds(fds)
}
