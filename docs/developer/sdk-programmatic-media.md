# Programmatic Node/Python media

G6-064 is in progress. Binary framing and managed Rust helper routing are
implemented; Node/Python media APIs are still being implemented. The
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
and MCP responses. Its local framing profile is `C6M1`. All integers are unsigned
and big endian:

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | `C6M1` |
| 4 | 1 | Request `0x01`, response `0x81`, packet `0x82`, closed `0x83`, failure `0xff` |
| 5 | 4 | Nonzero request correlation ID |
| 9 | 4 | Metadata length, at most 16 KiB |
| 13 | 4 | Payload length, at most 16 KiB + 12 bytes |
| 17 | metadata length | Metadata for the typed adapter |
| following | payload length | Exact binary payload |

The payload bound matches the Rust SDK's maximum complete RTP packet. Framing
does not rewrite RTP sequence, timestamps, marker, extensions or payload bytes.
Metadata is opaque at this layer. The next adapter must validate its schema,
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

The managed helper now accepts `media_attach` with an explicit loopback TCP
endpoint and a private one-use 64-character lowercase hex token. It rejects
remote/wildcard addresses, port zero and malformed tokens before connecting.
The helper sends a request with correlation ID 1 and metadata
`{"profile":"connect-sdk-media/1","token":"..."}`. The private listener must
verify it and reply with the same metadata in a response with ID 1 and no payload.
The helper verifies that response before starting the media actor. Tokens are
private IPC state; no account credential is sent on this channel. No unsafe raw
descriptor conversion or platform-specific socket filename is needed.

Following requests have strictly increasing IDs beginning at 2. Their metadata
is `{"timeout_ms":4000,"command":{...}}`. Typed commands are `describe`,
`send_rtp`, `send_clock`, `receive` and `stats`. Connection commands include a
`binding` object containing `conversation_id`, `session_id` and `connection_id`;
stream commands additionally name `stream_id`. `send_rtp` includes
`datagram_sequence` and carries the complete RTP packet as its binary payload.
Other commands refuse binary bodies. Clock reports preserve 64-bit NTP as a
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

The [routing evidence](../evidence/gate-6/sdk-media-ipc-routing.json) covers real
EdDSA/ES256 listener attachment, authentication/custody refusals, 16 blocked
receives with live control, bounded detach and server/runtime drainage. It does
not qualify language media interfaces or decoded media on this IPC path. Those
interfaces, bounded language queues, cancellation and the app-owned
encoding/decoding utility path remain to be implemented and verified.

Completion still requires actual Node/Python identified audio, decoded video
through the documented utility path and typed data, plus slow consumers, limits,
helper death and original cleanup. Candidate packaging and the complete
platform/version/facade matrix remain G6-069/G6-070.
