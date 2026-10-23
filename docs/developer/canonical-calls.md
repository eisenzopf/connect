# Signed canonical Rust calls

The [canonical native runtime](canonical-runtime.md) exposes typed calls on
`connect_client::canonical::CanonicalClient`. They use the enrolled Device,
credential and pinned server signing key from `CanonicalClientConfig`.

Persist the operation ID, message ID and normalized intent before a mutation.
`create_conversation`, `close_conversation`, `send_message`, `report_read` and
`participant_change` take caller-owned logical IDs. Reconnect and replay those
same IDs after cancellation, timeout or an interrupted connection. Every attempt
gets a fresh signed envelope ID and timestamp. A changed intent conflicts; the
client does not replace the operation ID to conceal uncertainty.

Creation requires an explicit tenant. Conversation create/close replies retain
the shared authorized Conversation DTO and its original mutation receipt in the
final ACK. Replaying creation after close returns the original receipt and current
closed state, with both opened and closed observations. Lists return the shared
page DTO and cursor, checked against the ordered opened/closed facts.
`send_message` checks the accepted operation, Conversation and message IDs against
the receipt. `participant_change` supports add/update/remove and checks completed
operation state, the receipt and the Participant change notice. `report_read`
checks the completed operation result against its compact ACK; the returned
recorded state can be delivery when read reporting has not been enabled.

`operation_get` returns the observed state, including `unknown`, with no invented
completion. `history` returns canonical message observations with Participant IDs,
not fabricated HTTP Identity mappings or membership epochs. The returned cursor
is opaque. `conversation_snapshot` and `subscribe_conversation` use canonical
payloads; subscriptions retain their original request correlation for live facts.

For other installed commands, `request(payload, scope)` returns `ReplyBatch`.
`request_with_cancel` adds explicit cancellation. Each batch ends at its own signed
ACK or error. The collector checks `in_reply_to` and Conversation, Session,
Connection and operation scopes. Only create/list may assign Conversation IDs when
the request has none. The batch has at most 128 frames and 2 MiB of encoded JSON;
each envelope remains bounded by the 1 MiB protocol limit. One absolute deadline
covers submission and every reply, limited by the original credential and server
pin expiry. A continuing stream of ready frames cannot renew that deadline.

Signed frames with other request correlations go to a separate inbox, also bounded
at 128 frames and 2 MiB. `receive` drains these retained observations before reading
new wire frames. Drain live events between calls; there is no detached reader.
Queue overflow, inconsistent scope, incomplete responses, partial-frame failures,
cancellation after submission and timeout retire the physical peer. They do not
prove that a mutation failed. Reconnect, recover the same operation, and resume
subscriptions from the last fully processed cursor. Already-cancelled calls and
invalid local inputs do not submit or retire an otherwise healthy peer.

`ReplyBatch::ack()` checks the terminal frame and maps signed business refusals to
sanitized application errors. Such a refusal leaves a fully consumed peer usable.
An ACK is response-batch completion only. Inspect a receipt or operation state for
logical completion; media readiness and asynchronous effects need their own proof.
The shared handler now terminates formerly fact-only replies (Conversation get,
operation recovery and Participant changes) with a correlated completion ACK.
History, message, read-report and subscription batches retain their existing ACKs.

The [scoped qualification](../evidence/gate-6/canonical-calls.json) records actual
production runtime cases and controlled signed post-auth stream failures. The
controlled streams qualify collector behavior only; enrollment and authentication
are proved by the production listener tests. The [default init/serve bootstrap](canonical-bootstrap.md) now uses this runtime.
This does not migrate the earlier `ConnectClient`, implement auth refresh/step-up,
install remaining command/media families, or close Gate 6 acceptance. Its checks
are local macOS arm64 only.

[Default command and application SDK usage](application-sdk.md) explains the
strict input/config formats and the separate HTTP application interface.

`CanonicalClient::fetch_record` fetches an inline native vCon using a
`CanonicalRecordSelector` and exact Conversation/Session scope. It verifies
schema, selected UUID and SHA-512 over the exact authorized bytes. A conditional
digest belongs to that caller's current projection. See
[native record retrieval](native-record-retrieval.md) for authority, limits and
remaining discovery/reference work.
