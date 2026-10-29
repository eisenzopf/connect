# Python reliable native data

`Stream.media.send_data()` and `receive_data()` use the original Client's
negotiated native data Stream and the separately authenticated binary helper
channel. Construction or operation restoration from saved IDs acquires no media
custody. The Client must keep the helper that negotiated the Stream alive.

Payloads are immutable `TextData(str)`, `JsonData(Json)` and `BinaryData(bytes)`.
They preserve the negotiated `text`, `json` and `binary` distinction; the facade
does not silently change a Stream's codec. Text is strict UTF-8, binary is exact
bytes, and JSON retains its typed value. JSON values compare by value even when
wire whitespace or number spelling differs. Payload representations omit content.
JSON is bounded to 32 levels/100,000 values, finite numbers and integers from
`-2**63` through `2**64-1`.

Each payload can contain up to 65,536 encoded bytes. Empty text/binary and JSON
`null` are valid. The server may enforce a lower configured message limit: the
default is 16 KiB. Full 64 KiB requires explicit host
`limits.message_bytes = 65536`. IPC metadata, including mutation extensions,
has its own 16 KiB limit. Invalid payloads, audiences and oversized metadata are
refused locally before starting the helper.

Persist the original message ID, operation ID, explicit participant audience,
Connection/Stream IDs and payload before calling. IDs identify original intent;
current server authority decides every submission or explicit replay. The example
below assumes `connection` is the original negotiated data Connection and
`recipient` came from the authorized Conversation's participant view.

```python
from connect_sdk import (
    DataAcceptance, DataPayload, Connection, MessageId, MutationOptions,
    NonEmpty, Operation, OperationId, ParticipantId, RequestOptions, StreamId,
)

async def publish(connection: Connection, recipient: ParticipantId,
                  message_id: MessageId, operation_id: OperationId,
                  payload: DataPayload) -> Operation[DataAcceptance]:
    # The application has already persisted this exact original intent.
    stream = connection.streams.by_id(StreamId("app-owner-events")).media
    return await stream.send_data(
        message_id, NonEmpty(recipient), payload,
        MutationOptions(operation_id, RequestOptions(4000)),
    )
```

The returned operation reference is small control metadata. Its existence does
not assert completion. Persist `operation.reference_json()` and explicitly await
`operation.wait(options)`. A completed `DataAcceptance` reports the original
message/Connection IDs and `boundary == "core-queue-admission"`; it is not remote
delivery, consumption or playback. A fresh Client can restore that reference with
`client.operation_kinds.data_send` and observe its original status under current
authority. The ordinary Conversation `message_send` kind refuses its full
Connection scope, and the data kind refuses an ordinary Conversation-only scope.
No payload bytes enter JSON control, CLI, HTTP or MCP responses.

Choose unique logical Stream labels for each publisher in the selection catalog.
For a bidirectional pair, publish on distinct source and target labels, subscribe
to the other publisher's label, then receive on the matching negotiated receive
Stream. Reusing one label for two publishers is ambiguous and fails closed;
a Participant selector does not remove a duplicated logical-label collision.
The [installed consumer](../../sdk/tests/python_native_server_consumer.py)
shows the complete public Connection offer, selection and both-direction data
journey, with original intent files written before publication.

`await stream.receive_data(options)` returns `ReceivedData` containing
`message_id`, `operation_id`, `from_participant` and a typed `payload`. Payload
`.kind` is `text`, `json` or `binary`, and `.value` contains the captured value.
`await client.media.data_stats(options)` returns exact Python integer counters;
reliable-data stats are separate from RTP counters.

There is one receive flight per scoped Stream across cloned handles. Cancelling
a local wait keeps that original flight owned; the next wait can consume its
original result once. A second concurrent reader on the same Stream is refused.
The language adapter retains at most 16 data receive flights/results (at most
1 MiB of payload), sharing the 16-request channel window with media packets.
It refuses additional reads rather than draining into an unbounded hidden queue.
Rust's original inbox separately bounds each Stream to 32 messages, all Streams
to 256 messages/1 MiB, and reports overflow as `resync-required` until a new
physical binding is explicitly negotiated.

A data-send deadline or cancellation after writing preserves the original
`OperationKey` with `uncertain` certainty. It never replays bytes or restarts the
helper. Before writing, the error is `not-submitted`. Normal asyncio task
cancellation raises `SdkCancelledError` with the same original key. Recorded
failures retain their validated original observation; foreign response keys,
scopes or references fence media with original mutation uncertainty. Control
remains independently usable. Closing the Client joins media actors and its
original helper; no automatic reconnect transfers native custody.

[Scoped evidence](../evidence/gate-6/python-reliable-data.json) records the clean
installed wheel, real production listener and independent Rust Store byte/effect
checks for EdDSA/ES256. The local component suite also covers limits, cancelled
reads and sends, helper death, malformed replies and bounded cleanup. The
[decoded-language evidence](../evidence/gate-6/language-decoded-media.json) adds
identified Python/Node audio/video through the documented app-owned utility and
requalifies native data/current-authority/cleanup with the rebuilt helper.
G6-064 implementation is complete in that recorded scope. The complete installed
platform/facade matrix and Gate 6 acceptance remain open.
