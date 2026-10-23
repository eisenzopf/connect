# Conversation record storage

G6-027 is in progress. The required end state is specified by [Connect records](../../contracts/connect-records.md). The scoped storage increment is recorded in `patches/rvoip/gate6-conversation-records.patch`; preparation uses the complete source-lock patch chain.

`rvoip_core::store::VconScope` carries exact Tenant, Conversation and Session ownership. It is data, not an identity, membership, audience grant or delivery receipt. Storage operations require it explicitly:

- `put(&scope, canonical_bytes)` stores one immutable body under that ownership.
- `get(&scope, &handle)` returns `None` for an unknown or foreign location and refuses a content hash that differs from the actual stored bytes.
- `list_for_session(&scope)` returns handles for that exact scope.
- `list_for_conversation(&tenant, &conversation)` uses both index keys. It does not serialize the reserved vCon federation group field.

Session finalization derives scope from Core's actual Conversation tenant and Session binding. Memory storage publishes body and ownership in one map entry. Its checked counter cannot wrap or partially publish a failed put. Colliding logical IDs in different scopes do not share access. Debug output redacts scope identities and record content.

The optional Postgres Core-store bridge applies the same ownership predicates and verifies the digest of returned bytes. Compiling this feature and running its schema-layout test do not qualify live database behavior. Its live test requires an explicit isolated database lane.

These low-level operations do not decide which private consultation, transcript, dialog, participant appearance or media reference a caller may see. Public gateways must use current per-source audience projection and retain delivery protection through the actual response. A URL or content hash does not grant access. Capture, retention, redaction/deletion revisions, protected concurrent retrieval, explicit finalization failure states and canonical fetch integration remain required G6-027 work. The storage increment does not close that task.

The existing public wire and HTTP boundaries remain [Connect UCTP](../../contracts/connect-uctp.md) and [the event HTTP reference](../../contracts/openapi-events.json). This increment adds no public HTTP route and does not make a raw store lookup an authorized public retrieval endpoint.
