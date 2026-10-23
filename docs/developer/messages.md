# Addressed and typed messages

A message is a durable conversation fact with a frozen audience. The caller's
current credential supplies the sender. `to` is either a nonempty set of
**participant IDs in this conversation** or `"all"`. `"all"` resolves active,
authorized members when the message is accepted. It does not mean future members.
The sender always belongs to the accepted audience. A narrow credential can send
to one allowed peer even when other people belong to the conversation.

```json
{
  "op": "send",
  "request_id": "saved-intent-001",
  "conversation_id": "conversation-id",
  "message": {
    "to": ["participant-id"],
    "content_type": "application/octet-stream",
    "body": "AP9hAA==",
    "body_encoding": "base64",
    "label": "sample",
    "attachments": [],
    "in_reply_to_msg": null,
    "msg_id": "client-message-id"
  }
}
```

Body-only wire requests are rejected. `body_encoding` is `utf8` or canonical
padded `base64`; content bytes must fit the configured message limit. Recipient
sets and attachment sets are canonicalized before computing a logical intent
digest. Duplicates are invalid. `msg_id` is optional; absent IDs are assigned by
the server. Message IDs are globally unique and cannot adopt another message.
Acceptance returns the durable message ID, not a delivery/read acknowledgment.

Rust's `send_message`, TypeScript's `sendMessage`, and Python's `send_message`
accept this draft. The text `send` convenience methods construct UTF-8
`text/plain` content with explicit `to: "all"`. Rust exports the draft, audience,
encoding and attachment types from `connect_client`.

History and inbox return content type, encoding, label, accepted timestamp,
sender participant ID, original participant/epoch audience, attachment references
and optional reply correlation. Context and task recall preserve typed metadata
for binary/structured content instead of presenting it as plain text. Missing
sources remain explicitly unavailable; timestamps come from recorded acceptance.

Keep the same `request_id` and complete input after losing a response. Matching
replay returns the original receipt without adding events or jobs, including
after backup/restore and late membership changes. Changed inputs conflict.
Current credentials, recipient ceilings and the sender's original membership
epoch still apply. Leaving and rejoining does not regain an earlier receipt or
private history. An explicit history share grants reading, never execution of
an old send.

The writer commits the message, recipient epochs, journal events, attachment
references, selected provider jobs and receipt atomically. A storage failure
acknowledges none of them. Inbox consumers persist their cursor after processing;
notifications remain hints and an offline member catches up from the journal.

`in_reply_to_msg` must be readable by the sender and every selected recipient.
A private question cannot be widened by a reply. Its owner can explicitly share
that question before a new reply addresses the additional member. Read projections
hide parent and artifact references that the current viewer cannot access.
Exported model context also intersects the model participant and delegate scope.

Attachment references contain `id`, `content_type`, and `size_bytes`. They must
match an existing ready, unexpired artifact that the sender and each recipient
can currently read; client-selected URLs are rejected. G6-034 separately extends
artifact creation/download links to the message audience and implements delivery
observations. This increment does not establish private artifact upload semantics
or new signed download links.

Only selected connector participants get provider jobs. Plain-text connector
adapters receive eligible UTF-8 text; unsupported binary/attachment deliveries
have terminal `unsupported` state and `unsupported-message-content` refusal,
without external dispatch. Such a refusal fails a tracked provider effect and
does not block later supported conversation turns. Correlated provider answers
address the original question sender and retain the parent message ID. They do
not broadcast to unrelated members or assistants.

Independent provider ingress uses the route's required, immutable `incoming_to`
policy, configured by trusted composition. Explicit participant sets and `"all"`
use the same writer and current authority. Each new incoming `"all"` message
resolves the current members; replay of an old provider mutation never expands
its frozen audience. A route must be enabled and its current credential must
permit the send. External caller/provider IDs do not establish authority.

Canonical `message.send` and `message.history` payload adapters feed the same
writer and authorized query. A claimed `from` participant must equal the current
credential's participant under the acceptance transaction. History filters apply
before pagination; parent cursors require current access. Canonical transport
dispatch and profile negotiation remain G6-037, rather than a second core handler.
The current HTTP route is `POST /v1/operations`; see
[OpenAPI request and response shapes](../../contracts/openapi-messages.json).

Fresh state uses schema 31. Older candidate databases, including schema 26,
are refused unchanged. Preserve them and their matching binary/backup and use a
new explicit state path; there is no automatic conversion or reset.

Run `python3 scripts/check-gate6-messages.py` in an environment with
`scripts/requirements-gate6.txt` installed for isolated local qualification.
Its report records source hashes, test binaries, actual HTTP samples, failed
candidate attempts and the remaining profile/transport/provider boundaries.
