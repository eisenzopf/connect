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

The current 64 local SDK/helper/HTTP/declaration functions pass with 203 guarded
inputs unchanged. Cases include empty evidence, unavailable configuration,
foreign scope, duplicates, malformed state/provenance/epoch/time/channel and
collection overflow. The installed package matches the generated distribution.
Local Rust compilation is skipped because the retained Mac linker ENOSPC
prevents a reliable native build.

The Ubuntu fixture now configures the application service explicitly, reports a
delivery through the independent recipient's public application client, reads
that fact through Node after helper restart, and refuses the read after current
credential revocation. This is a separate `receipts-observed` scenario in
`node-controls/3`: four seconds per SDK request, 15 seconds per phase and the
original 90-second two-algorithm function cap. Compilation, both-algorithm
behavior and original drains are pending; the fixture is not recorded as a pass.
See [the evidence record](../evidence/gate-6/node-receipts-sdk.json).
