# Connect durable record storage

G6-046 installs a Service-owned implementation of Core's `VconStore` over the
Connect SQLite Store. `Runtime::new` supplies this adapter instead of Core's
process-local memory backend. Store owns storage, exact scope and original
mutation facts; Service owns canonical vCon and source-manifest validation and
redaction. Neither component grants permission to view a source.

Schema 75 adds the record index and original-mutation facts. A document UUID is
bound to its Tenant, Conversation and Session, exact canonical bytes, complete
source manifest, two SHA-512 hashes and revision. Session/Conversation linkage
stays in the local index; the reserved vCon group field is not used. The Session
and its tenant must already exist. Identical creation recovers the original live
revision-one handle. Changed bytes, manifest, ownership, a revised record or a
tombstone conflict rather than overwrite or resurrect data.

`connect:vcon/<uuid>` is a local index location. It is not an HTTP endpoint,
media link or permission. UUID and location selectors must agree when both are
present. Arbitrary URLs are never fetched. Unknown or foreign scopes return no
record. Reads validate the exact body and manifest hashes and retain the same
record reservation in Core's `VconRead`. The reservation survives the SQLite
read and must remain owned alongside current audience authority through actual
response IO. It is not a substitute for audience authorization.

Each record has its own fair cross-process read/write reservation, separate
from Identity and Conversation authority. Mutation waits for retained reads;
unrelated records can proceed. Local deletion and source-selective redaction
compare the expected revision, body hash and manifest hash under the exclusive
reservation. Their original operation ID, intent fingerprint, result and time
commit with the changed record in one SQLite transaction. Failed transforms
leave both bytes and mutation history unchanged. Exact retries recover the
historical fact, including redaction replay after a subsequent deletion; changed
intents and operation ID reuse across mutation kinds conflict. This is trusted
backend idempotency; public mutation replay still needs current authority.

Redaction uses Core's canonical source/dependency transformation. Deletion
removes live body and manifest columns and preserves a local tombstone and
original mutation facts. Old live handles cannot retrieve the deleted record.
No provider erasure or physical disk sanitization is claimed. Snapshot integrity
includes both new tables, and online backups retain records and mutation facts.
Record/body/manifest/mutation limits are finite; deletion has a reserved terminal
slot after the redaction-history limit.

This backend remains one G6-046 component. The separately documented
[durable Session outcome path](session-record-outcomes.md) now freezes terminal
intents and runs an owned canonical-server finalizer without requiring Core's
automatic recorder. External Core projection itself still does not initialize
that recorder. The [native reader](native-record-retrieval.md) composes source authorization
with the retained backend reservation. Public readiness discovery and guarded
HTTP/TLS record delivery are installed; see [HTTP record retrieval](http-record-retrieval.md).
Public redaction/deletion is described in [record-mutations.md](record-mutations.md).
Named installation retention and its owned worker are described in
[record-retention.md](record-retention.md). Captured-media references remain
G6-046 work. Default capture remains metadata-only; no unfiltered public record
route is added.

Issued live HTTPS references can also select a record in a new
`recording.vcon-fetch` request. The Service resolves only metadata in its own
bounded registry before deriving current source policy; Conversation/Session and
any supplied UUID must agree. The backend resolves the original locator again
under the actual Store tenant/resource scope. Unknown, expired, foreign or
mismatched URLs are local misses and cause no network requests. A URL does not
carry its issuer's source permissions into this new read. The optional condition
compares the newly authorized projected digest. Direct HTTPS GET keeps its own
original exact view/revision/expiry checks; selecting a record does not reactivate
an expired reference.
