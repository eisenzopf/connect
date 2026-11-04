# Node negotiated packets and reliable data

`NodeClient.media`, `Connection.media` and `Stream.media` use the original
Client's negotiated native binding over a separately authenticated binary helper
channel. Restoring IDs or operation references grants observation, not another
Client's media custody. Keep the original Client and helper alive.

`await connection.media.refresh(options)` returns immutable negotiated Stream
IDs, media, directions, codec declarations, source-clock support and Stream
revision. `stream.media.sendRtp(new SdkRtpPacket(sequence, bytes), options)` sends
a complete RTP packet without rewriting its header, extensions, padding or
payload. Construction captures the caller's bytes; returned bytes are copies.
The complete-packet bound is 16,396 bytes. `sendClock(sequence, report, options)`
uses `bigint` for the exact unsigned 64-bit NTP value. A successful RTP or clock
submission means `boundary === "local-media-queue"`, not remote playback.

`receivePacket(options)` returns a discriminated `kind`: `rtp`, `clock` or `gap`.
RTP has captured bytes, parsed headers and payload. Clock reports retain exact
NTP values; gaps report `droppedPackets` as `bigint`. `client.media.stats(options)`
also uses `bigint` counters and bounded numeric queue sizes. Applications own
encoding/decoding, capture and rendering; this packet interface does not supply
a desktop device or codec implementation.

Reliable payloads are `SdkTextData(string)`, `SdkJsonData(Json)` and
`SdkBinaryData(Uint8Array)`. They preserve the negotiated `text`, `json` and
`binary` distinction. Text is strict UTF-8, JSON is captured and recursively
frozen, and binary input/output is copied. Debug inspection omits content.
Each payload permits at most 65,536 encoded bytes, including empty text/binary
and JSON `null`. JSON allows finite JavaScript numbers, plain objects, dense
arrays and at most 32 levels/100,000 values. Exact 64-bit integers should use an
application string representation; JSON does not accept `bigint`.

The default server message limit is 16 KiB. Full 64 KiB requires explicit host
`limits.message_bytes = 65536`. Binary IPC has its own 16 KiB metadata budget;
mutation extensions count toward it. Invalid inputs are refused before media
attachment. Constructing a NodeClient itself starts its owned control helper.

Persist the original payload, message ID, operation ID, explicit audience and
Connection/Stream IDs before submission. For example, with a negotiated data
Connection and a recipient from the authorized Conversation participant view:

```typescript
import {
    SdkTextData, type SdkConnection, type MessageId, type OperationId,
    type ParticipantId, type StreamId,
} from "@rudeless/connect";

async function publish(connection: SdkConnection, recipient: ParticipantId,
                       streamId: StreamId, messageId: MessageId,
                       operationId: OperationId) {
    // The application has already persisted this exact original intent.
    const operation = await connection.streams.byId(streamId).media.sendData({
        messageId, to: { first: recipient, rest: [] },
        payload: new SdkTextData("owner event"),
    }, { operationId, timeoutMs: 4000 });
    const reference = operation.referenceJson(); // Persist before waiting.
    const accepted = await operation.wait({ timeoutMs: 4000 });
    return { reference, accepted };
}
```

The small operation reference is control metadata. Only a completed
`SdkDataAcceptance` declares `boundary === "core-queue-admission"`; it is not
delivery or consumption. A fresh Client can restore it with
`client.operations.restore(reference, client.operationKinds.data_send)` and
observe its original status under current authority. The ordinary Conversation
`message_send` kind refuses a data operation's full Connection scope.

`receiveData(options)` returns original `messageId`, `operationId`,
`fromParticipant` and a discriminated typed `payload`. `client.media.dataStats`
has separate exact `bigint` counters. Choose unique logical Stream labels for
each publisher in the selection catalog. Publish bidirectionally on distinct
source/target labels, subscribe to the other label, and receive on the matching
negotiated remote Stream ID. A Participant selector cannot disambiguate duplicate
publisher labels. The [installed consumer](../../sdk/tests/node_sdk_server_consumer.mjs)
shows the complete offer, selection, saved-intent and recovery journey.

The channel owns at most 16 requests and 16 queued responses. A scoped Stream
has one receive flight across cloned handles. Cancelling or timing out a caller
wait retains that original flight/result for its next wait; a concurrent second reader is
refused. At most 16 reliable-data receive flights/results retain 1 MiB of
payload. Rust separately bounds its data inbox to 32 messages per Stream and
256 messages/1 MiB overall; overflow requires explicit resynchronization on a
new physical binding. RTP inboxes remain separately bounded to 32 per Stream,
256 packets/4 MiB overall. The adapter refuses overflow instead of hiding it in
an unbounded application queue.

A receive reply can arrive after its caller deadline even when the helper had
already captured the packet. The receive owner keeps that original IPC result;
a later reader does not submit a replacement read. The helper retains its
original timeout. RTP, exact unsigned-64-bit clock reports and reliable data
follow this rule; ordinary outbound request waits keep their existing behavior.
The [deadline regression evidence](../evidence/gate-6/node-media-receive-deadline-retention.json)
records the original failure and 21 passing controlled media cases. Actual
installed app verification of this correction remains pending.

The Rust live-media inbox expires queued RTP/source-clock packets at 200 ms.
`receivePacket` reports those discards as `gap` before current queued packets;
`overflowDrops` includes capacity and age discards. Reliable data is unaffected.
This bounds local queue age only, and requires a helper built with the correction;
the previously sealed 3732254 candidate retains its earlier receive behavior.

The Node adapter also expires a completed RTP/clock result retained after a
cancelled wait once its local slot residence reaches 200 ms. The next reader
gets a frozen `gap` with `droppedPackets === 1n`, with no second read request or
helper restart. Already reported gaps preserve their exact counts, and reliable
data remains recoverable after that delay. Adapter expiration appears in the
returned gap; `client.media.stats` still reports Rust inbox counters. This needs
the updated Node package; the sealed 3732254 Node archive predates it.

Cancellation or a deadline after a data frame was written preserves the
original `OperationKey` with `uncertain` certainty. Before writing it remains
`not-submitted`. The adapter never replays bytes or restarts the helper.
Recorded failures retain validated original observations. Malformed or foreign
replies fence media while original control remains independently usable. Closing
the Client owns the listener, accepted socket, reader, writer and original helper.
Unsupported helper media versions refuse before attachment.

[Scoped evidence](../evidence/gate-6/node-programmatic-media.json) records 125
component checks, 48 negative type constraints and a clean installed package
against the real local EdDSA/ES256 listener. Full 64 KiB text/JSON/binary passed
both directions, independent Rust Store byte/effect checks, recovery, replay,
changed intent, foreign custody and cleanup. The
[decoded-language evidence](../evidence/gate-6/language-decoded-media.json) adds
identified independently decoded PCMU/VP8 through the documented app-owned
utility and requalifies data/current-authority/cleanup with the rebuilt helper.
G6-064 implementation is complete in that recorded scope. The complete installed
platform/facade matrix and Gate 6 acceptance remain open.


For explicitly selected native video feedback, `stream.requestKeyframe(options)`
submits a hint for the current receive epoch. `stream.receiveVideoFeedback(options)`
returns an immutable helper-owned reference. Consume it once with
`stream.consumeVideoFeedback(hint, options)`: an expired, superseded or delayed
reference returns no permit. A returned permit's `isFresh()` must still be true
immediately before forcing the next fresh source frame. Its deadline charges
the complete consumption RPC round trip against the original Rust lifetime;
response receipt never starts a new 200 ms lifetime. Cloned/foreign references
cannot be consumed. `SdkRtpPacket.videoEpoch` exposes matched receive history
without changing the original RTP bytes. Set `native_video_feedback = true`
in the original helper SDK TOML, before its `[control]` table, to opt in. The
default is false. Server selection must confirm the profile; configuration
alone cannot establish feedback authority. Integrated transport qualification
is tracked in the Gate 6 evidence.
