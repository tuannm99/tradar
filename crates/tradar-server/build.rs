//! Compiles a tiny fixture gRPC service (`tests/fixtures/greeter.proto`)
//! used only by `tests/rpc_grpc.rs` to drive a real server + reflection over
//! a loopback TCP connection -- see `docs/backlog/grpc-rpc-methods-*.md`.
//! Harmless to run even when the `grpc` feature is off: it just produces
//! code nothing then includes.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::var("OUT_DIR")?;
    let descriptor_path = std::path::PathBuf::from(&out_dir).join("greeter_descriptor.bin");

    // Pure-Rust compile (no `protoc` binary needed on the build machine),
    // same reason the real gRPC connection path uses `protox` too.
    let fds = protox::compile(["tests/fixtures/greeter.proto"], ["tests/fixtures"])?;

    use prost::Message;
    std::fs::write(&descriptor_path, fds.encode_to_vec())?;

    tonic_build::configure().compile_fds(fds)?;
    Ok(())
}
