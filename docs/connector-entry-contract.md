# Connector entry contract

The narrow pre-Gate 4 extension boundary is now a registered **message/context
connector**. `Connector::descriptor()` declares address validation, conversational
context and SMS policy. Trusted server composition registers the descriptor before
claiming jobs; applications cannot register adapters through the wire API.

The store applies these contracts without a provider-name branch for a new chat
adapter. Context requires both the submitting actor and route delegate to retain
`context_view` permission and conversation access. Selection and outcome are
journaled with the actual connector name. Uncertain/failed conversational turns
block later turns until reconciled. A definite rejected turn may be discarded by
a currently authorized operator; uncertain effects cannot be discarded or retried.

Descriptors are immutable within a process. Register the same descriptor on every
restart. Legacy Telnyx and Vapi descriptors preserve existing route semantics.
Missing registration fails closed; it must never silently become a one-way route.
Route changes still require current operator authority and conversation membership.

`entry_connector.rs` supplies an independent `research-assistant` fixture via the
connector API and composition only. Its authorized context reply, delivery receipt,
replay and restart use ordinary Connect storage and SDK operations. The same test
suite injects a failed result commit and proves readiness/failure recovery without
duplicating the remote effect.

This closes R07's narrow context-bearing fixture criterion. The existing Vapi
telephone DTOs, native Vapi audio adapter and Telnyx callback translation remain
explicit provider integrations. This is not the full generic voice connector kit.
A new chat host uses this contract; a new telephone/media provider must first define
its normalized lifecycle contract in the Gate 5 connector work. Do not add another
provider-name switch to core context dispatch.

The full manifests, packaging templates, independent contributor exercise and
protocol conformance remain the separately promised Gate 5 deliverables.
