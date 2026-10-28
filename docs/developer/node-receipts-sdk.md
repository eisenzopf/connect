# Typed Node message receipts

`conversation.receipts(messageId, options)` reads current per-recipient/channel
evidence through the public Rust SDK. Message acceptance and delivery facts are
separate results. Each fact preserves participant identity, membership epoch,
channel, delivery identity, state, provenance and observation time. A reported
`read` state retains its provenance; it does not establish human action.

```ts
const facts = await conversation.receipts(savedMessageId, { timeoutMs: 4000 });
for (const fact of facts) {
  console.log(fact.state, fact.channel, fact.membershipEpoch, fact.provenance);
}
```

The fixed SDK configuration must explicitly include its application read
service. There is no inferred endpoint or fallback. Node advertises
`message-receipts` only when the Rust configuration supports it; the feature
still requires current authority for the requested read. Missing configuration
returns `unsupported`.

Rust owns current authorization, pagination and the one shared request deadline.
It discards accumulated observations if a later page refuses or is malformed.
The helper only serializes that public SDK result. Node additionally validates
the exact Conversation/message, bounded collection, unique recipient/epoch/
channel/delivery groups, enum strings, provenance and integer bounds. Returned
arrays and facts are immutable. Metadata grants no transport or media custody.

The current 82 local SDK/helper/HTTP/declaration functions pass with 213 guarded
inputs unchanged. The installed public package also passes all 14 HTTP tests.
The HTTP error test checks that actual status remains independent of an
`unauthorized` response body: HTTP 500 or 200 cannot count as authorization refusal.

The original Ubuntu Node 22 and 26 matrix artifacts at `5d984b57` both show the
actual delivered native/client_reported fact before revocation. Both failed a
later assertion that expected only authentication: the SDK correctly returned
cancellation after revocation closed its native owner. Its read guard remains.

The `node-controls/5` candidate independently requires the same fact through the
public named application HTTP API, then actual HTTP 401/403 after revocation.
The owned SDK read must return no data with precise refusal or closed-transport
certainty. Canonical HTTP must also refuse; SQLite must still contain exactly
one original message and operation. Original deadlines and joins remain required.
Current native compilation/execution is pending. Local Rust linking is skipped
for retained ENOSPC. See [the correction](../evidence/gate-6/node-receipt-authority.json)
and [the original source-bound results](../evidence/gate-6/node-native-5d.json).

The exact `e732fcef` Node22.23.4/26.12.1 Ubuntu artifacts now pass the complete
`node-controls/5` function under both algorithms. Named HTTP sees the same native
delivery fact, then actual 401/403; the owned read returns no data after current
revocation, canonical HTTP refuses, and one original message/operation remains.
All required8helpers/6validators/2servers join per matrix. The newer helper3,
incoming and graceful-stop changes remain pending. See
[the source-bound results](../evidence/gate-6/node-native-e732.json).
