# 014 — Gate 6 clean contract version

2026-10-15; G6-003 / G6-010. The owner explicitly permits breaking pre-alpha API,
SDK and schema changes and rejected legacy API compatibility work.

The target is UCTP envelope 2, native ALPN uctp/2 and application profile connect/2.
The existing eight-byte RTP datagram header remains version 1 because its byte
format does not change. HTTP/3 ALPN remains h3. The SDK/server package release
version is independent and is not chosen as v0.2 automatically.

Canonical communication verbs dispatch once through Connect's service/store.
Existing Connect request dialects and route-ID-oriented public SDK methods may
be removed. No old-client matrix, downgrade path or compatibility facade is
required. The server refuses unsupported envelope/profile/schema versions
explicitly before acting.

At the new schema boundary, inspect the installation version before opening
mutable state. If incompatible, stop with backup and explicit reinitialization
instructions. Do not silently delete/migrate the installation or widen credential
permissions. A Gate 6 backup must restore with its matching schema/runtime;
automatic v0.1 conversion is not required.

Retain product capabilities and adapt meaningful behavior tests to the new API.
Old serialized field-shape tests are not release obligations. Preserve historic
v0.1 release evidence with its original source and limits.

## Implemented state boundary

Schema 23 requires explicit artifact audiences in persisted `artifact_begin`
requests and task effects. Schema 22 candidate state is refused rather than
interpreting those old intents under a new audience policy.

Gate 6 state uses SQLite schema **23**. Opening a different schema stops before
mutable configuration or cleanup. `migrate` does not convert historical state.
The current runtime restores only a matching Gate 6 backup. A restored connection
row preserves its ID/lineage but requires fresh adapter observations; it does not
restore a physical call.

Keep the existing installation and its matching binary. Stop that server and use
its supported backup command to preserve the database, keys, configuration and
artifact blobs together. Keep provider secret references and their protected
vault separately as described by the existing operator recovery instructions.
Do not edit `PRAGMA user_version` to bypass refusal.

Initialize Gate 6 in a **new directory**. The source-checkout command is:

```sh
bash scripts/prepare-dependencies.sh
cargo run --locked -p connect-server --bin connect -- init --directory .local/gate6
```

Choose an unused directory; initialization refuses an existing installation.
New installation and credential IDs are intentional. This does not import old
conversations or continue an old provider engagement. Keep the prior installation
and verified backup until its owner chooses their disposition.

For a matching Gate 6 backup, the public CLI is:

```sh
connect backup --config STATE/operator.toml --server-config STATE/server.toml --output BACKUP
connect restore --backup BACKUP --directory NEW_STATE
```

`STATE`, `BACKUP` and `NEW_STATE` are explicit local paths; both destination
directories must be new. The backup includes artifact blobs and installation
recovery material. Treat it as private. Provider credentials in an external vault
remain external; a copied reference is not a copied secret.

Verification: `cargo test --locked -p connect-store` covers current state and
backup invariants. The server's `first_run_schema_boundary` and
`artifact_recovery` tests exercise the public CLI and file recovery.


## Recipient receipt storage (schema 24)

G6-034 replaces the unused candidate receipt table with bounded, immutable
per-recipient, per-channel state facts and epoch-scoped optional preferences.
Original recipient authority, client-reported read provenance, monotonic delivery
projection and current preference filtering apply across facades. Prior schema 23
state is refused without modification; use backup and explicit reprovisioning.
No historical conversion is implied by the fresh initialization SQL. See
[attachment and receipt behavior](../developer/attachments-and-receipts.md).


## Durable transition ledger (schema 25)

G6-035 adds frozen transition authority snapshots, protected worker/runtime and
resource fences, attributed decisions/evidence and explicit cleanup. Legacy current
receipt families cannot collide with canonical operation IDs. Opening an older
candidate never upgrades or resets it. Backup and explicitly provision a new state
path. [Runtime operation contracts](../developer/operations.md) and
[HTTP/OpenAPI](../../contracts/openapi-operations.json) describe the current control
surface; physical coordinators and canonical wire dispatch remain separate tasks.

### Snapshot and event recovery candidate, schema 26

The G6-036 candidate adds a journal observed-time field and durable retention floor.
It stores no automatic migration of a schema-25 installation. Read-only snapshots
capture their watermark transactionally and use bounded process-local, currently
authorized pages. New event positions use padded authenticated encryption with
`ring` (already locked upstream) so global sequence positions and digit lengths are
not visible to recipients. The dependency is now explicit in `connect-store`.
Transient observation source handles are protected backend composition, bound to
current connection generations and explicit frozen audiences; they grant no model
or provider operation authority. Canonical wire and provider/media integrations
remain their later implementation tasks. This historical foundation did not close
G6-036; the final schema-27 evidence is recorded below.

### Journal provenance and resource versions, schema 27

The schema-26 foundation remains historical partial evidence. The current fresh
candidate adds stored session/file/call/SIP/context state revisions and immutable
journal provenance captured at the original insert. Session topology revision is
separate from state version. A transaction-scoped core writer context binds logical
request correlation to the authenticated identity and is empty at every commit;
task invocation IDs retain their own namespace. All eight current journal insertion
sites are inventoried in `contracts/gate6-event-writers.json`. This closes the
missing-version fallback; later reads cannot relabel old facts with current state.
OpenAPI and language SDK shapes are explicit. The October 16 complete unchanged
source run qualified this G6-036 scope: 533 Rust tests, 211 TypeScript tests,
29 Python tests, 165 contract vectors and three actual HTTP/OpenAPI samples.
See [the recorded evidence](../evidence/gate-6/events.json). Physical graph/provider
normalization and canonical dispatch retain their separate tasks. Older
candidates, including schema 26, are preserved and refused unchanged.

### Authentication before Conversation selection

The G6-026 runtime implements unscoped Identity/Device authentication before a
Conversation exists. `auth.session.participant_id` is therefore optional; when
present it requires an envelope Conversation ID. Authentication does not invent
Conversation membership or Session presence. This corrects the inherited baseline
payload's global-Participant implication without changing the six-noun model.
The schema generator, typed Rust decoder and 168 shared vectors agree. See the
[canonical peer guide](../developer/authenticated-peer.md) for the implementation
and its remaining qualification boundaries.

### Issuer and verification-key facts, schema 28

Schema 28 adds immutable actual credential issue times, Device revisions, owned
EdDSA/ES256 key records and explicit credential-to-key bindings. The trusted
administrative provisioning ledger owns enrollment and permanent key/Device
revocation; validation never guesses an issue time or enrolls a payload-selected
key. Key ownership, validity and retirement history survive snapshots, and the
logical snapshot digest includes both new tables. Schema 27 and older candidates
are preserved and refused unchanged. See [peer credential records](../developer/peer-credentials.md).
Production owned authority/retirement policy and canonical listener installation
remain G6-037 work; a current key lookup is not a durable or future execution grant.

### Cooperative authority protocol, schema 29

Schema 29 is a fresh-state binary protocol boundary for owned authority fences.
It changes no table shape beyond the schema marker. Matching Store writers
coordinate through private per-Identity and per-Conversation file locks before
entering SQLite transactions. A schema-28 binary must refuse this state instead
of opening it and bypassing those locks. Schema 28 and earlier candidates remain
preserved and refused; explicitly initialize a new path. This does not qualify
all writer integration or production canonical policy; see the
[authority fence implementation boundary](../developer/authority-fences.md).

### Installation ownership protocol, schema 30

Schema 30 is a fresh-state binary boundary for installation ownership and runtime
observation epochs. It adds no table shape beyond the marker. Schema-29 binaries
must refuse this state instead of opening it without the new ownership protocol.
Prior candidate databases remain unchanged: preserve a verified backup with its
matching binary, then explicitly initialize a new path. No conversion is added.

Store owns the exclusive kernel file lock. Same-process ordinary handles share
one owner; new servers and offline repair acquire exclusively. Administrative
opens never acquire/adopt the runtime or perform startup maintenance. Each new
owner has a fresh observation epoch, including same-process restarts and restored
copies. Server Store handles borrow that owner's lifetime; admitted SQL units
retain a permit through completion while retired handles cannot admit more work.
Connection epoch and legacy video startup maintenance use bounded exact-target
resource write fences. These facts do not by themselves qualify the canonical
listener, complete current reader policy or network/media lifetime permits.


### Provider dispatch custody, schema 31

Schema 31 records an immutable enqueue-time file identity and single-use claim
stamp for every provider message job. Dispatch admission is durable and monotone;
backup digests include these rows. The worker runtime, exact source authority,
body, frozen context and previous chat must match again at last-mile admission.
A claim alone is not evidence that host network I/O was admitted.

Same-file recovery cancels an unadmitted claim as a known local non-effect.
An admitted attempt remains uncertain. A restored copy cannot prove that its
pending snapshot was never submitted later by the original file, so restored
pending jobs also remain uncertain and cannot create another message. No retry,
conversion, downgrade or backward API compatibility is introduced. Schema 30 and
earlier candidate state is preserved and refused; explicitly initialize a fresh
path with this binary and retain verified backups with their matching binary.

The protected connector API now transfers a non-wire DispatchPermit into message
adapters and their actual HTTP/TLS socket. Permit clones share one creation
admission; pending I/O retains its resource guards and runtime role until socket
cleanup, even after caller cancellation. This qualifies selected Telnyx/Vapi
message creation only; status, voice/SIP, response delivery and the full canonical
listener/current policy remain separate work. See [provider dispatch](../developer/provider-dispatch.md).


The normalized status continuation retains schema 31 and its ownership/fence
protocol without adding a table. The protected extension API now requires owned
read-only custody for reconciliation. It cannot admit creation when forwarded or
cloned. Metadata inspection is explicitly separate from transport admission;
current credentials, membership, audience, registered status capability and the
runtime owner are checked again after the file wait. Actual pending I/O retains
its resource guards and installation owner through cleanup. This does not expand
built-in provider capability declarations or close the complete canonical policy.
See [owned provider reconciliation](../developer/provider-reconciliation.md).

### Shared control receipts, schema 32

Ordinary control mutations now use the `operations` key and result, with immutable
`operation_control_receipts` metadata for frozen authorization and task origin.
Fresh initialization drops the old ordinary receipt table; existing candidate
state is refused rather than converted. Stable ownership remains tenant/Identity
plus logical ID. Task internal wire receipts are private to their task/grant and
cannot be queried through core status. Local identity creation uses provisioning.
Message acceptance carries a checked local-mutation receipt and a get reference;
completion of that local mutation does not certify recipient/provider delivery.
See [control receipts](../developer/control-receipts.md) for evidence and remaining
G6-037 boundaries. The source-lock remains at its 61 recorded upstream patches.

### Conversation control, schema 33

Fresh initialization adds conversation opening time, idle policy configuration,
canonical metadata and complete close reasons. Old candidate state is refused,
including schema 32, with explicit backup/reset instructions. Create/close use
shared typed intents and ledger receipts; list is a bounded current projection
with an opaque owner/credential/tenant/scope/filter-bound cursor. Canonical create
has a server-selected CID admitted before SQL. No metadata field or role is
silently discarded or treated as authority. JCS serialization now supports the
validated metadata domain. See [conversation control](../developer/conversation-control.md)
for behavior, evidence and remaining production-dispatch boundaries. The
source-lock remains at its 61 recorded upstream patches.

### Membership control, schema 34

Fresh initialization adds participant removal reasons and an explicit shared
receipt origin (member epoch, nonmember administrator, invitation target or
unbound nonconversation command). Schema 33 and earlier candidate state are
refused; no automatic conversion is supplied. All ten membership actions use
the ordinary control engine. Canonical Add performs explicit admission under
current manager and named capability authority; invitation acceptance remains
a separate explicit workflow. This preserves the frozen Add payload instead of
inventing consent or expiry fields. Roles remain labels.

Capture the original actor epoch before mutation; a leave/rejoin cannot revive
an old receipt. Only the original same-epoch completed self-removal receipt
is recoverable while inactive. Persist removal reasons in views and journal;
clear them on readmission. Business rejections and infrastructure rollback
retain distinct recovery semantics. Public queries use current authority and
personal invitation pages use encrypted credential-bound fixed-high-water
cursors. See [membership control](../developer/membership-control.md) and its
HTTP reference for evidence and precise remaining boundaries. No upstream
source changes are added by this family; source-lock remains at 61 patches.

The canonical subscription continuation keeps schema 34. It adds no stored fields:
opaque in-process continuation facts, exact read proofs and common-writer custody
operate over the existing journal, snapshot and audience model. See
[canonical subscriptions](../developer/canonical-subscriptions.md).
