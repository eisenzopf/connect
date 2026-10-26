# Live-media fixture conformance v1

Profile `connect-live-media-fixture-v1` executes the reusable
`connect-media-conformance` crate against a contributor's `Fixture` factory and
its actual public rvoip `ConnectionAdapter`/`MediaStream` implementation. The
harness does not accept a caller-provided list of test results. The factory
provides isolated retained backend state, trusted request construction and
instrumentation of the actual encoder/decoder and effect journal. Instrumentation
and factory requests are fixture controls; they must never become a production
admission API.

The profile is explicitly simulated, outbound-only PCMU: 8 kHz mono, RTP payload
0, 160 samples/20 ms. The source encodes the positive 4096 marker; the sink decodes
the independently supplied negative 8192 marker. Source frames are independently
decoded by the harness's real G.711 decoder. The selected fixture has four-frame
queues and eight retained receipts with no eviction. These marker values are
test data, not a model response, human speech or real provider delivery.

Every check opens a separate backend, receives a bounded current host permit,
invokes actual transport behavior and has a five-second task deadline. A timed
out check is aborted and its handle observed. A failed or timed out check proves
no clean adapter teardown. Positive teardown checks must join original workers.
The checks cover:

- Dormant/staged origination, complete original intent, single effect activation,
  current and read-only custody, and no stopped-original resurrection.
- Actual duplex PCMU encoding/decoding, RTP progression, source receiver ownership
  and reservation rollback, paired queue termination and unchanged post-End metrics.
- Stale generation rejection at the actual decoder after buffering.
- Current revocation, expiry and source backpressure. The transport must emit
  `Cancelled` itself for lost custody, before the harness requests drain;
  backpressure emits `Failed` and cannot stand in for an authority fence.
- Closed-queue terminal fallback after liveness retirement, exclusive ownership
  and same original effect/Connection ID across an in-process adapter restart.
- Explicit unsupported media/control refusal, immutable runtime matching and
  refusal at receipt capacity.
- Cancellation-safe cleanup that retains the original worker handle for retry.
- Actual registration on a Core with a fail-closed inbound gate, refusal of all
  inbound routes and terminal closure of admission on host cancellation.

The template's `BrokenCustody` factory intentionally replaces the harness's
current/read-only permit with its own authority. It drives the real adapter and
must fail the staged-custody and revocation/expiry checks. Its report is an
expected refusal, never a passing fixture. The report runner rejects missing,
duplicate, failed or unknown checks and changed source inputs. Build diagnostics
stay in ignored private state; shared reports contain source hashes and named
checks, not credentials or raw media.

The public message/PCM lifecycle harness remains a separate profile,
`connect-connector-fixture-v2`. This live transport component profile does not
qualify inbound support, stock Connect application workflows, an installed kit,
network media, provider/model-host/browser/Linux/remote behavior, latency/capacity
or process-crash persistence. Those require their own assigned behavior evidence.
