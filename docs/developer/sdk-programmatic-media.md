# Programmatic Node/Python media

G6-064 is in progress. The binary framing foundation below is implemented;
helper routing and Node/Python media APIs are still being implemented. The
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

Remaining implementation includes authenticated private channel attachment to
the same managed Rust Client, typed negotiation/packet/lifecycle actions,
bounded language queues and cancellation, and the app-owned encoding/decoding
utility path. The workspace forbids unsafe code, so raw inherited-FD ownership
must not be implemented with unchecked descriptor conversion. An explicitly
authenticated loopback attachment is under consideration. None of these
remaining paths is qualified by the framing component tests.

Completion still requires actual Node/Python identified audio, decoded video
through the documented utility path and typed data, plus slow consumers, limits,
helper death and original cleanup. Candidate packaging and the complete
platform/version/facade matrix remain G6-069/G6-070.
