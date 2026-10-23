# Conversation control (G6-037, in progress)

This schema-33 checkpoint is historical. The [schema-34 membership continuation](membership-control.md)
extends shared control; its original report below remains unchanged.

Schema 33 gave canonical `conversation.create`, `conversation.list` and
`conversation.close` the same typed Store writer as HTTP, native control and the
public Rust, TypeScript and Python helpers. This is an explicitly composed
canonical listener slice. Production signed native client/listener installation,
subscriptions and full Gate 6 acceptance remain open.
The previous [schema-32 receipt checkpoint](control-receipts.md) is historical;
its report is retained unchanged. The current reference is
[OpenAPI conversation control](../../contracts/openapi-conversations.json).

`create_conversation` in Rust/Python and `createConversation` in TypeScript accept
`tenant_id`, `policy`, `idle_close_secs`, `metadata` and `initial_participants`.
Each initial appearance has an Identity and role. The `create` convenience method
constructs an explicit persistent intent with the default participant role.
Only authenticated tenant membership supplies authority. The HTTP/native tenant
field may be null to derive the current tenant; a supplied tenant must match.
Canonical create requires the tenant. The caller is included if absent. Duplicate
explicit initial Identities are refused; roles are labels and grant no credential.

Metadata is canonicalized with RFC 8785 JCS, including UTF-16 key ordering and
numeric formatting. It is limited to 16 KiB and depth 16; unsafe integral numbers
above the JavaScript safe-integer range are refused, including exponent notation.
Nested duplicate JSON metadata keys are refused by the shared DTO. Metadata is
retained in SQLite, not diagnostic Debug output. Canonical wire validation also
precedes shared typed input. No old create body or old candidate database is
converted; back up incompatible state and explicitly initialize a fresh path.

Create chooses a server CID before SQL and obtains its Conversation writer,
creator Identity writer and target Identity readers. Concurrent same-ID retries
resolve the committed CID before admission. Close obtains the existing
Conversation/member writers. Signed proof retains exact typed command, source
credential, enrolled Device/key, resource coverage and runtime ownership. The
same proof survives blocking SQL and signed response submission; no reader
reentry or lock upgrade occurs beneath the admitted writer. Source expiry after
a SQL wait rolls back without reserving the logical ID.

Business effects, journal facts and the shared operation receipt commit together.
Changed same-ID intent conflicts. Create and close receipts identify a completed
local mutation and an exact `operation.get` reference. SDK helpers require that
reference without automatic retry. A conversation replay projects current state
under the original membership epoch and frozen recipient ceiling while retaining
the original receipt; a create replay after close reports the same CID as closed.
It does not reopen the conversation. Request status and operation get use this
same authorized projection. This does not certify media or provider completion.

Close options contain optional `expected_revision`, `reason_code` and `reason`.
The precondition is checked inside the mutation after receipt lookup, so exact
replay does not conflict with the revision produced by that close. Already-closed
conversations accept omitted reason assertions; a supplied different reason
conflicts. Unknown or unfinished Session, voice-call, SIP-leg or transition work
blocks closure. Physical media retirement is not inferred from these SQL facts.

List accepts tenant, participant, Identity, state (`open` by default, `closed` or
`all`), since and until filters. Times are RFC3339 UTC-compatible timestamps.
Only current live member conversations are visible; participant descriptors also
respect the caller's recipient ceiling. Cursor ownership binds current Identity,
credential, tenant, scope and filter. The encrypted opaque token expires within
30 seconds or the credential's remaining lifetime and retains a fixed row
high-water; newly created rows do not enter the subsequent pages. Each page
projects current state and membership. This is not an immutable snapshot.

A page caps requested entries at 100, encoded entry data at 384 KiB, source scopes
at 1024 and canonical fact replies at 126. The cursor never advances past an
unreturned entry. A terminal entry requires both `conversation.opened` and
`conversation.closed` facts because the targeted opened schema has no state
field; create replay of a closed conversation uses the same pair. List ends with
an `ack` and optional `next_cursor`. Every fact retains correlation and carries
its actual CID. No private omitted-entry count is exposed. An invalid cursor retains only the
authenticated Identity reader for its bounded validation refusal, never a
Conversation grant or a returned page.

Ephemeral conversations default to 60 idle seconds and accept 1–604800 seconds.
Idle starts from creation or the latest accepted message/session update and runs
only when all Session/call/SIP/transition work is terminal. A supervised server
worker checks at one-second intervals in batches of at most 32. It reacquires
current per-Conversation/member writers, rechecks eligibility in the transaction
and commits `idle timeout` plus journal facts atomically. Persistent conversations
retain an explicitly supplied idle configuration but never auto-close. Worker
failure marks operational readiness unavailable; owned shutdown is bounded.

For current sources, run `scripts/check-gate6-membership-control.py` in the
verification environment. The historical schema-33 report records the exact source and binary hashes, HTTP/OpenAPI samples,
selected Rust/SDK regressions and unresolved acceptance boundaries. SQL-adjusted
idle/time fixtures prove Store policy only; the separate runtime case waits for
actual one-second idle closure through the public native SDK. Local fixtures use
no provider account or remote deployment.
