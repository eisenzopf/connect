# Independent live-media contract fixture

This is an explicitly simulated, in-process media transport. It implements the
public rvoip `ConnectionAdapter` and `MediaStream` interfaces and the Connect v2
connector manifest. It uses no Connect Store/service internals, provider account,
SIP call or wire authentication shortcut. Its generated speaker marker is fixture
audio; it is not an AI response or human speech.

From prepared repository sources:

```sh
bash scripts/prepare-dependencies.sh
cargo test --offline --locked --release \
  --manifest-path examples/media-connector-template/Cargo.toml \
  --target-dir .local/gate-6/media-connector-template-target \
  -- --test-threads=1
bash scripts/format.sh --check
```

The source dependencies select the repository's pinned public rvoip interfaces.
Exported kit component qualification and stock application journeys have
separate G6-068 evidence. The protected server composition uses `LiveTransport::new` with this
same `Arc<Adapter>` and its `LiveMediaLifecycle` implementation; server-side
startup/teardown tests qualify that narrow installation path separately. This standalone component is not qualification of a network
transport, provider, browser, remote host, capacity, latency or crash persistence.

`Adapter::open` owns one fixture backend. The caller retains `Remote` across an
in-process adapter restart; it models the remote provider's retained original
effects independently of Connect's authoritative journal. New ownership waits for
old worker cleanup. The eight-entry receipt table never evicts an old operation
to make uncertain work appear new. A stopped original ID is observed and cannot
silently produce another route.

Trusted composition supplies `Admission` through rvoip's typed `OriginateContext`,
including the original operation ID and a `DispatchPermit`. These options have no
wire/serde representation. The host must retain its real current policy fences
and resources in the permit's authority implementation. IDs, manifest metadata,
codec declarations and fixture input never supply conversation authority.

Origination is dormant and replays the same complete intent and Connection ID.
Activation requires one current effect admission, across every permit clone, and
starts at most one owned worker. Read-only custody cannot create an effect.
Media keeps checking current host custody after buffering. Generation-aware
`PeerMediaFrame` entries acquire their delivery guard after dequeueing and retain
it through the actual decode; stale speaking generations are discarded.

The profile supports outbound PCMU only: 8 kHz mono, 160 encoded bytes per 20 ms
frame. Its source generates a known positive marker; its sink independently
decodes the caller's marker. Inbound admission, video, data, messages, DTMF,
renegotiation, transfer and wire principal authentication are explicitly
unsupported. Queue capacity is four frames; source backpressure stops the route
with a failure observation instead of growing memory or reporting healthy media.
Quality remains unmeasured.

The guarded Core consumes the atomic orchestrator event queue directly. This
outbound-only adapter never creates or emits inbound routes, and its manifest
never advertises inbound admission. Its atomic handoff guarantee cannot be used
to infer inbound support. A second subscription cannot steal the first receiver.

`Adapter::cancel` permanently closes admission under the same lock as origination
and activation; `drain` also closes admission. End and drain fence the route and observe the original worker handle. Cancellation
of a drain keeps that handle for the next cleanup attempt. Drop requests stop;
it is not clean-teardown proof. A saturated or closed terminal queue uses the
installed rvoip lifecycle fallback after liveness is retired.

The reusable component harness exercises staged/replayed intent, real duplex PCMU decoding,
receiver reservation rollback, stale buffered generations, current revocation and
expiry, bounded queues, terminal fallback, in-process restart, unsupported media,
receipt capacity and cancellation-safe cleanup. None of these checks turns a
manifest or an ACK into proof of delivery to a real external participant.


Run the shared conformance runner against this independent package:

```sh
python3 scripts/check-media-connector-contract.py \
  --target-dir .local/gate-6/media-connector-template-target \
  --output .local/gate-6/media-conformance-positive.json
python3 scripts/check-media-connector-contract.py \
  --target-dir .local/gate-6/media-connector-template-target \
  --negative-control --output .local/gate-6/media-conformance-negative.json
```

Both runner invocations exit zero when their respective expectation is verified.
The negative report has `fixture_passed:false` and `status:expected-refusal`.
Its factory replaces current/read-only custody before calling the real adapter;
it cannot populate harness check results. Standalone `cargo test` executes both
factories. The optional `conformance` feature enables the executable reporter;
normal adapter-library consumers do not include the harness.

To adapt a genuine transport, implement the public rvoip interfaces and
`LiveMediaLifecycle`, then implement the fixture-only factory and instrumentation
in a separate module/package. Preserve the harness's exact permits and operation
IDs. Keep the actual codec, queue and unsupported profile declarations truthful;
use a separate profile for other codecs or inbound behavior. Read the
[executable profile](../../contracts/connectors/live-fixture-v1.md). Component
reports complement the stock canonical-host startup/cleanup tests; current public
application workflows and exported kit source snapshots retain separate G6-068
evidence.

## Public native host

`native_host_manifest()` is a separate `SignedNativeEndpoint` profile for an
owned public SDK host. It describes PCMU audio, end control, no text/context
export and five bounded device frames per direction (four backend frames plus
one bridge frame). Native substrate queues are governed by the SDK separately. `manifest()` continues to describe the
`InProcessAi` backend adapter. Use that transport manifest when installing the
adapter in a Core; the signed native host manifest does not satisfy that binding.

The companion host uses `connect_client::canonical::PublicPcmuBridge` and its
`PcmuDevice` interface. It enrolls its own signed QUIC Endpoint through public
administration, explicitly accepts the Session and selects permitted audio
sources through `PublicNativeAudio::subscribe_audio`. The adapter is the local
synthetic source/sink; its local lifecycle permit does not grant Conversation
rights. All actual communication uses the enrolled native SDK peer and current
server media policy. No Store access or dispatch replacement is needed.

The bridge owns the native peer and local device, paces one pending PCMU frame,
stops on malformed frames or failed IO, and cancels both resources before bounded
cleanup. Its `drain(&self)` retains the original worker across cancellation of the
wait. Check both cleanup fields in the terminal report; Drop alone proves only a
stop request. See `crates/connect-server/tests/connector_media_stock.rs` for the
current executable public-client workflow. This simulated local device is not a
live provider, model, browser or remote-host qualification.
