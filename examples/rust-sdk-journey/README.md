# Independent Rust SDK consumer

This separate Cargo workspace depends on `connect-client`, Serde, serde_json and
Tokio. It has no Connect server, service or Store dependency. The verification
runner also copies these exact sources into a fresh workspace outside the
repository and inspects its resolved dependency graph.

The source checkout currently needs `scripts/prepare-dependencies.sh` and the
same recorded upstream patches as the root workspace. This is a public source
consumer, not a claim of published-package installation or packaged dependency
readiness. Publication remains separate.

Build from the repository root:

```sh
bash scripts/prepare-dependencies.sh
cargo build --release --offline --manifest-path examples/rust-sdk-journey/Cargo.toml
```

The executable takes a private JSON case file. `media` creates a Conversation,
sends one separately identified private Message, recovers its original operation,
receives an actual incoming Session invitation, accepts it, negotiates both
native SDK peers, selects streams, exchanges RTP and independently decodes
identified PCMU audio and VP8 video with FFmpeg. It ends the Session and joins
both clients. The source intent and decoded artifacts stay in private fixture
state. No live device capture or provider delivery is inferred.

`transition` submits a saved typed handoff or device-move intent and recovers its
captured result through a fresh SDK client. Its bounded line protocol permits
the real-server fixture to provide independently owned public native media legs
while the operation is preparing. Those legs retain their physical ownership;
the transition controller never obtains it from the returned metadata.
`recover` performs a separately authorized original-reference read after an
unsuccessful transition. The case contains the full original intent before IO.
No case accesses server internals or silently resubmits a mutation.

`replacement_peer` owns the separately signed physical target for a saved
`connection.replace` operation. Its private case supplies the original command,
current operation root, native Opus offer, native configuration path and bounded
original encoded packets. It allocates the target, validates both negotiation
legs, answers the parameter callback, sends paced RTP/source clocks and records
received native datagrams before joining its client. The current mode is mono
48 kHz Opus with 20 ms packets for at most 30 seconds. Its `passed` observation
does not infer operation completion, decoded media or readiness from an answer
or clock report; the controller and independent decoder prove those separately.
Capture its diagnostic stdout under ignored `.local/` beside the private case
and output. See the [replacement procedure](../../docs/developer/native-physical-replacement-peer.md).

The isolated server tests are ignored by default because they require a concrete
built consumer. To run only those tests against this example build:

```sh
CONNECT_GATE6_EXTERNAL_SDK="$PWD/target/release/connect-external-rust-journey" \
cargo test --release -p connect-server --test gate6_canonical_runtime \
  external_public_crate_ -- --ignored --test-threads=1 --nocapture
```

Use the actual absolute binary path when setting a custom Cargo target directory.
These tests use local fixtures for EdDSA and ES256. They do not select a remote or
provider rehearsal. Detailed evidence and unresolved profiles are recorded in
`docs/evidence/gate-6/rust-sdk-external.json`.
