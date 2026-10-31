//! Compile the exact prepared production selector without the SIP unit-test graph.
// include! keeps repository formatting from traversing prepared upstream files.
mod production_duplex {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.deps/rvoip/crates/sip/rvoip-sip/src/media_stream/guarded/duplex.rs"
    ));
}
