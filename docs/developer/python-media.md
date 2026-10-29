# Python native packet interface

G6-064 is in progress. The current Python packet interface implements negotiated
native RTP, source-clock reports and local queue gaps through the same Client's
separate binary helper channel. It does not supply desktop capture/render or a
decoded frame API. Reliable typed data and the encoding/decoding utility path
are still being implemented; G6-064 is not complete.

The helper must advertise both native Rust media support and
`managed-media-ipc-v1`. Python exposes `native-programmatic-media` only for that
combination. Unsupported helpers refuse before media attachment; there is no
HTTP or historical PCMU fallback. Deploy the compatible helper on PATH as
described in [the control guide](python-sdk.md).

Negotiate on the original Client before using media. An ID, journal snapshot or
saved operation restored on a fresh Client does not acquire another physical
peer's media bindings:

```python
import struct
from connect_sdk import (
    Client, ConversationId, SessionId, ConnectionId, StreamId, NonEmpty,
    OfferedStream, OperationId, MutationOptions, RequestOptions, RtpPacket,
)

options = RequestOptions(4000)
async with Client("client.toml") as client:
    session = client.conversations.by_id(ConversationId("room")).sessions.by_id(SessionId("session"))
    offered = await session.connections.offer(
        ConnectionId("original-connection"),
        NonEmpty(OfferedStream(StreamId("audio"), "audio", "sendrecv", NonEmpty("g.711-mu"))),
        MutationOptions(OperationId("original-offer"), options),
    )
    connection = (await offered.wait(options)).value
    negotiation = await connection.media.refresh(options)
    audio = connection.streams.by_id(StreamId("audio")).media
    # Complete RTP packet: the application owns sequence/timestamps and encoding.
    raw = struct.pack("!BBHII", 0x80, 0, 1, 160, 0x12345678) + bytes([127]) * 160
    accepted = await audio.send_rtp(RtpPacket(1, raw), options)
    assert accepted.boundary == "local-media-queue"
```

Queue admission is not delivery, decoded audio or playback. `RtpPacket` captures
immutable complete RTP bytes and validates version/header, CSRC, extensions,
padding and the 16 KiB + 12-byte RTP limit. Its `header` and `payload` are views
of those exact captured bytes; debug output omits media contents.

`await audio.receive_packet(options)` returns one of:

- `RtpPacket`: the complete negotiated RTP packet, with its transport sequence.
- `ClockPacket`: a `SenderClockReport`, separate from RTP or readiness. NTP stays
  an exact Python integer and is transported as a canonical decimal string.
- `MediaGap`: the number of packets dropped by the local bounded queue, separate
  from network loss observable through transport/RTP sequence numbers.

`send_clock(sequence, report, options)` requires negotiated source-clock support.
Reports must describe the application's actual source; a report does not prove
that another participant received or rendered media. `client.media.stats()`
returns exact counters and current bounded native queue sizes.

The channel permits at most 16 pending exchanges and uses bounded socket write
buffers. Read handles for the same Stream share one pending receive. Cancelling
a read wait leaves its original packet/future owned for the next wait; concurrent
reads on that Stream refuse with conflict. Cancelling an unsent packet refuses
before submission; cancelling a sent wait reports uncertainty without replay or
restarting the control helper. Clocks and local queue gaps never become fake RTP.

Use `async with Client(...)` or `await client.close(options)` to own all original
helper/socket tasks. `client.media.close()` retires and joins its local binary
channel while retaining control; it does not silently submit Connection/Session
mutations. Use their ordinary end operations with stable IDs for those changes.
Incomplete bounded cleanup remains an explicit error and retains its owned work
for a later close; a partial binary write is never restarted on a reused channel.

The [scoped evidence](../evidence/gate-6/python-media-packets.json) records 206
component checks, 40 negative type constraints, and an installed Python 3.14.7
wheel using default PATH lookup against real local EdDSA/ES256 listeners. The
installed journey proves negotiation and RTP/clock local queue admission,
original control/restart/current authorization and original process/server joins.
It does not qualify decoded language media, reliable typed data, Node media,
Linux/remote profiles or the full platform/package/parity matrix.
