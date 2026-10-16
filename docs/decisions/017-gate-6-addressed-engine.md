# 017 — Addressed messages in the reusable engine

G6-018 implements the engine boundary specified in
[the audience contract](../../contracts/connect-audiences.md).
The implementation is recorded in `patches/rvoip/gate6-addressed-messages.patch`
and consumed through `dependencies/source-lock.json`.

`Orchestrator::send_message_to_conversation` resolves `All` into a fixed
participant list before persistence. Explicit recipients must be nonempty and
known to the conversation or its accepted live connections. Fanout selects only
those participants' connections and checks the current policy before dispatch.
`send_message_to_connection` applies the same rules, including a check that the
connection belongs to this message's conversation. An adapter receives the typed
Message with its content type, attachments, reply reference and origin intact.

A gateway with a typed inbound message calls `receive_addressed_message`. The
connection supplies authoritative conversation/sender attribution. The method
rejects mismatched attribution and records connection provenance; it never
reduces a typed message to text. Text-only adapter ingress still records an
explicitly resolved audience and uses the same admission hook. Provider-specific
translations and capability limitations remain work in G6-044.

A reply must be visible to its sender and cannot widen the original audience.
An explicit application sharing operation is required to widen it.
`message_for_participant` and `list_messages_for_participant` check the original
resolved audience and current policy. The metadata-only projection removes body
bytes and attachment URLs. References to no-longer-visible original messages are
removed from projections.

`Config::message_policy` is the integration hook for durable application grants,
credential revocation, membership epochs, retention and bounded receipt routing.
The default checks intrinsic audience relationships without supplying application
credentials or durable epoch authority. Connect must install its store-backed
policy as part of G6-031/G6-033; this engine work does not qualify those tasks.
The raw store/history methods and engine event bus are trusted process-internal
surfaces, not public subscription endpoints. Gateways must apply the scoped
projections before returning content. A source page is bounded to 256 messages;
its participant projection may be empty with a continuation cursor. The public
Connect cursor and indexed durable filtering are assigned to G6-036.

Receipts carry message, recipient, optional connection, stage and an explicit
viewer set limited to the sender and that recipient. Each viewer is checked
against current policy. The policy hook routes the observation; it must not
broadcast it to every conversation member. Adapter send success is **accepted**,
not delivered or read. Read observations require current recipient authority.
These per-recipient observations replace unconditional engine delivery/read
broadcasts on these message APIs. Provider confirmations remain G6-034.

Applications authenticate callers before entering the trusted engine. Policy
implementations bound their calls and adapters fence external submission against
current grants. Durable mutation deduplication, uncertain external submissions,
atomic Connect fanout and provider callback normalization remain G6-033–G6-035.

Run `python3 scripts/check-gate6-rvoip.py --component messages` for deterministic
adapter tests and source-bound evidence. Real Connect, provider and transport
conformance remain separate qualification tasks.
