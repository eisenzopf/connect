# Programmatic Node/Python media

G6-064 implementation is complete in its recorded local scope. Binary framing
and managed Rust helper routing are
implemented; Python [packet](python-media.md) and [reliable-data](python-data.md)
APIs and the [Node packet/data interface](node-media.md) are implemented.
The [app-owned PCMU/VP8 utility](../../examples/sdk-media/README.md) supplies the
decoded file path. The
current control SDK's Stream and Connection observations do not acquire media
custody by being restored from IDs or operation receipts.

The original public Rust SDK owns negotiated media bindings. Its receive inbox
bounds each Stream at 32 packets and the entire inbox at 256 packets/4 MiB;
native sends use a bounded queue of 64. The language adapter must preserve those
bounds, current binding/direction checks and typed gaps. It must not create an
unbounded secondary queue to conceal a slow application.

The existing `media-stdio`/`NativeAudioClient` path is the earlier PCMU interface:
it accepts fixed 160-byte audio payloads and has a 4 KiB record limit. It does
not implement the negotiated audio/video/data contract for this task.

The new helper channel keeps media bytes separate from JSON control, HTTP, CLI
and MCP responses. Its local framing profile is `C6M2`. All integers are unsigned
and big endian:

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | `C6M2` |
| 4 | 1 | Request `0x01`, response `0x81`, packet `0x82`, closed `0x83`, failure `0xff` |
| 5 | 4 | Nonzero request correlation ID |
| 9 | 4 | Metadata length, at most 16 KiB |
| 13 | 4 | Payload length, at most 64 KiB |
| 17 | metadata length | Metadata for the typed adapter |
| following | payload length | Exact binary payload |

The payload bound preserves the full reliable-data contract. Complete RTP packets
still have the separate 16 KiB + 12 byte bound, enforced by the public packet
constructor. Framing does not rewrite RTP sequence, timestamps, marker,
extensions or payload bytes. Earlier `C6M1` framing is refused; there is no
legacy adapter.
Metadata is opaque at this layer. The typed adapter validates its schema,
formats, clock declarations and original Client's negotiated binding before any
media operation; a well-formed record is not an authorization grant. Framing
debug output includes lengths, never metadata or media contents.

One owned reader retains partial headers and bodies across cancelled application
waits. It checks all header bounds before allocating a body. Unknown profiles,
unknown record kinds, oversized lengths and truncated records fence the reader;
it does not scan for another magic value. Clean EOF is distinct from truncation.
The writer validates bounds before writing a prefix. A writer actor must finish
the original frame or retire its channel if an application cancels its wait;
restarting a partial write would corrupt the next boundary.

Managed capabilities include the private `managed-media-ipc-v2` version marker.
A language facade checks it alongside native media support before attachment;
unsupported versions refuse explicitly rather than selecting another interface.
The managed helper accepts `media_attach` with an explicit loopback TCP
endpoint and a private one-use 64-character lowercase hex token. It rejects
remote/wildcard addresses, port zero and malformed tokens before connecting.
The helper sends a request with correlation ID 1 and metadata
`{"profile":"connect-sdk-media/2","token":"..."}`. The private listener must
verify it and reply with the same metadata in a response with ID 1 and no payload.
The helper verifies that response before starting the media actor. Tokens are
private IPC state; no account credential is sent on this channel. No unsafe raw
descriptor conversion or platform-specific socket filename is needed.

Following requests have strictly increasing IDs beginning at 2. Their metadata
is `{"timeout_ms":4000,"command":{...}}`. Typed commands are `describe`,
`send_rtp`, `send_clock`, `receive`, `stats`, `send_data`, `receive_data`
and `data_stats`. Connection commands include a
`binding` object containing `conversation_id`, `session_id` and `connection_id`;
stream commands additionally name `stream_id`. `send_rtp` includes
`datagram_sequence` and carries the complete RTP packet as its binary payload.
`send_data` carries its typed payload in the binary body. Other commands refuse
binary bodies. Clock reports preserve 64-bit NTP as a
canonical decimal string. Received RTP, clock and local queue-gap results stay
distinct; counters that can exceed JavaScript's exact integer range are decimal
strings. A successful send means `local-media-queue`, not delivery or playback.

The actor routes through the same Client that owns the JSON control helper.
Binding lookup and packet operations retain the public Rust SDK's ownership,
direction, retirement and current-peer checks. A fresh Client restoring the
same saved operation does not acquire another Client's native negotiation.
There are at most 16 active media requests and 16 queued responses. Slow media
I/O has a separate path from control. `media_detach` on the JSON control channel
cancels and joins the actor, requests and socket writer; closing the managed
Client also owns that cleanup. An invalid record fences its channel.

Reliable data uses `format` equal to `text`, `json` or `binary`; text and JSON
bodies are UTF-8 and binary bodies are exact bytes. `send_data` also carries
`message_id`, an explicit nonempty `to` participant audience and
`mutation:{operation_id,extensions?}`. The caller persists this original intent
before submission. A returned `data_submitted` reference can be recovered through
the control helper's `data_send` operation kind; only its completed value declares
`core-queue-admission`. Neither the reference nor that admission means remote
delivery. Receive metadata retains the original message/operation IDs, sender,
format and binding; body bytes never enter JSON control output. Data counters are
separate from RTP counters. Empty text/binary payloads remain valid.

A server may configure a lower message limit: its default is 16 KiB. Full 64 KiB
operation requires an explicit host `limits.message_bytes = 65536` configuration.
The IPC limit does not raise that host ceiling or bypass current authorization.
A revoked peer may close before a signed refusal arrives; preserve the original
operation and uncertainty, without automatic replay. A fresh owner with that
revoked credential cannot authenticate. Restoring a data operation for control
recovery grants no native Stream custody.

The [v2 data evidence](../evidence/gate-6/sdk-media-ipc-data.json) records exact
64 KiB text/JSON/binary in both binary IPC directions against independent Rust
consumers/producers and the real server under EdDSA/ES256, original recovery,
one-effect replay, changed intent/current revocation/foreign custody refusals
and joined original owners. The existing attachment, 16 blocked receives,
independent control and drainage checks also pass on v2. The earlier
[routing evidence](../evidence/gate-6/sdk-media-ipc-routing.json) remains scoped
to its v1 source. Current Python v2 packet/data interfaces pass 217 component
functions and 43 negative type constraints. The installed native5 journey proves
full 64 KiB typed data in both directions, original operation recovery/replay,
independent Rust Store bytes/effects and original cleanup under EdDSA/ES256;
[Python evidence](../evidence/gate-6/python-reliable-data.json) records its scope.
Reliable data has a separate Rust inbox of 32 messages per Stream, 256 messages/
1 MiB overall. Python retains at most 16 data receive flights/results (another
1 MiB) and refuses overflow instead of draining into an unbounded secondary queue.
Publisher labels must be unique in the selection catalog; receive on the matched
negotiated remote Stream ID. Node has the same 16-flight/1 MiB retained data bound.
Its [scoped evidence](../evidence/gate-6/node-programmatic-media.json) records 125
component functions, 48 negative type constraints and a clean installed real
EdDSA/ES256 native journey: full 64 KiB typed data in both directions, independent
Rust Store bytes/effects, original recovery/replay/custody and joined resources.
The app-owned encoding/decoding utility path is documented and verified below.

The [decoded-language evidence](../evidence/gate-6/language-decoded-media.json)
closes G6-064's implementation verification: both Node and Python exchange
identified synthetic PCMU audio, independently decoded VP8 video through the
app-owned utility and full-size typed data under EdDSA/ES256. Slow-reader,
framing/size, cancellation, helper-death and cleanup checks retain their separate
component/production scopes. The utility documents supported formats and explicit
FFmpeg requirements; no desktop capture/render support is implied.

The same run exposed a retained journal fact bypassing credential revocation.
The original Rust SDK and managed helper now revalidate the same journal
permission and cursor with a finite server read before release. Original facts,
acknowledgements and mutation IDs are not replaced. The deterministic regression
fails before the fix and passes under both algorithms afterward.

Candidate packaging and the complete installed platform/version/facade matrix
remain G6-069/G6-070. Named remote/provider profiles, physical devices, motion,
latency and audiovisual synchronization are not qualified by this finite file
exchange. Gate 6 acceptance remains unqualified.
