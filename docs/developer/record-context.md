# Record sources in task and provider context

Implementation and verification are tracked under G6-046. This change adds record
sources to the current metadata outcome profile; it does not enable audio capture
or claim provider-side erasure. See the [HTTP context reference](../../contracts/openapi-context.json)
for the response contract.

A context owner needs `context_view` and independent `record_get` permission to
include records. A task or invocation must also explicitly include `record_get`
in its own operation ceiling. A backing account's wildcard does not supply that
missing task permission. An assistant's delegate credential needs its own current
permissions. A tenant reader role does not create Conversation membership or
private context rights.

Each ready record is an audience-bearing resource with its original UUID and
Session. Explicit Session history sharing retains the shared record floor under
the same sharing ID; revoking that share also revokes derived record access.
Record-only sharing does not confer live Session participation or private captured
Connection epochs. Nonadministrative erasure retains its separate admission.

Context selection intersects the owner's source policy, the delegate's policy and
every actual listener's current participant epoch. Core applies the same canonical
source pruning, dependency remapping and retention rules as fetch. Raw media URLs
are withheld. The Store owns current authorization; Service supplies the pure Core
serializer. A copied projection is data, never a storage or audience lease.

A record context item has its UUID as `source_id` and a `record` reference with:

- `uuid`, `session_id` and `revision` identifying the exact selected resource;
- `state`: `available`, `unavailable` or `deleted`;
- `content_hash`: the authorized unsigned projection's digest, or null;
- `valid_until`: the earliest visible retention cutoff, or null.

Raw whole-record and manifest fingerprints stay server-side. The body contains the
authorized canonical vCon only when available. Task recall keeps explicit absence
and budget notes rather than inventing content. Selection bounds aggregate input
processing and output bytes independently.

The canonical native assistant and chat delivery paths retain the original real authority interval through IO and
clips it to selected record retention. Before creation, export, observation or
reuse of an opaque provider instance, the host reconstructs the original selection
at its saved watermark. A changed record, redaction, deletion or current source
refusal cannot borrow an old receipt or projection digest. Historical delivery
observations remain facts; they cannot authorize another export or establish that
remote provider memory was erased.

Record mutation and retention use the Conversation writer before the record
writer. A retained delivery read therefore finishes before erasure confirmation;
fresh delivery rechecks the changed record. Chat reuse also binds the current
authorized record references, so an old remote chat identifier cannot replace a
fresh projection after redaction, deletion, or a visible retention boundary.

Local provider evidence is recorded separately for the
[public](../evidence/gate-6/record-context-public-provider.json) and
[private](../evidence/gate-6/record-context-private-provider.json) paths. Private
cases cover the actual listener's Record audience, explicit share revocation,
independent context permission, erasure, retention and pending acknowledgement
custody. Earlier complete cohorts exposed a return refusal after retention
expiry; its cause remains unresolved.

[Physical private-provider write evidence](../evidence/gate-6/auxiliary-record-provider-write.json)
now covers an unfinished record-bearing WebSocket frame. The original delivery
keeps its source and member authority while public erasure queues, and reading
the complete frame still requires its exact acknowledgement before erasure can
commit. A separate frozen-source-retention case closes the partial frame at its
original deadline, releases the queued writer and fails the operation without
cancellation; the existing public audio remains usable. Durable retention cleanup
commits independently after that deadline.

The valid32KiB context budget can expand beyond64KiB through nested JSON escaping.
Private initialization therefore uses the existing bounded512KiB provider-control
socket profile so its exact acknowledgement fits. The metadata budget, source
cutoff and five-second IO maximum remain unchanged. The local macOS fixture uses
12KiB trusted escaping metadata on an actual Session record and a test-process-only
TCP profile limited to its exact designated loopback port. Actual kernel queues
and a full negotiated segment prove the frame is unfinished; production socket
options are unchanged. Linux, captured media, provider-side erasure and remote
profiles are not qualified by this fixture. Mixed private captured-source
projections and the older Vapi media attachment remain unqualified. Gate 6
qualification is still in progress.

Schema 81 installs the record resource registration invariant. Older state is
refused without automatic migration, reset or downgrade. Preserve a verified
backup and matching binary before explicitly initializing another state path.
