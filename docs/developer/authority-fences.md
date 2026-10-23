# Owned authority fences (G6-037, in progress)

The [owned runtime continuation](owned-runtime.md) records current schema-31
startup ownership. Earlier evidence reports retain the schema/source state they
qualified; they are historical checkpoints.

Fresh state uses schema 31. Earlier candidate states are refused unchanged;
initialize a new explicit path. The marker prevents older binaries from opening
state whose matching writers must participate in the cooperative fence protocol.
This implementation is qualified only by the local macOS arm64 fixtures named in
[the evidence report](../evidence/gate-6/authority-fences.json). Production peer
policy and full authority-writer coverage remain open.

`Store::read_authority` derives the current credential owner and, for a
Conversation, all active member Identity scopes plus the Conversation scope.
It releases the SQLite mutex before waiting, acquires the sorted scope set,
then rechecks it. A changed membership set requires release and fresh admission.
The returned owned guard must survive verification, effects and delivery.
`authority_covers` checks the exact Store namespace and required scope set so
protected composition can reuse held authority without reentering a reader lock
behind a waiting writer. It does not validate a principal or signed request.
A value returned by credential/key lookup remains a snapshot, not a grant.

Each scope has an admission turnstile and a shared/exclusive resource lock.
A writer retains admission while waiting for existing readers and while writing;
new readers cannot pass its intent, including readers sharing an existing local
file descriptor. Unrelated Identity scopes can proceed. Duplicate requests are
collapsed with write dominating read, and acquisition order is deterministic.
Acquisition accepts at most 1024 requested scopes; member derivation refuses
oversized sets. Waits are bounded to at most 30 seconds (Store writers default to
two seconds), can be canceled, and release partial acquisitions on every error.
Busy/unavailable authority is an explicit retryable refusal; the admin HTTP
facade maps it to 503. An actual key-revoke request held behind a read guard
returns 503 without changing the snapshot, then succeeds with the same logical
mutation ID after release. Its request and responses are checked against OpenAPI. No SQLite or global grant mutex spans a delivery await.

The kernel owns lock lifetime. Each process shares read descriptors through weak
references; retaining the last guard retains the lock, and process death releases
it. The namespace is bound to the installation secret and observed
database device/inode/owner; file identity also unifies case aliases on
case-insensitive volumes. Each manager checks its original canonical database
path against that identity before admission. Opaque HMAC names contain no raw resource IDs;
namespace directories are mode 0700 and files are mode 0600, checked for owner,
privacy, regular-file shape and single-link ownership. Symlink aliases resolve
to one canonical database, blob directory and lock namespace; hard-linked database aliases are refused before opening SQLite.
Live database moves, replacement, manual lock-file removal and direct SQL writes
are unsupported during service operation. These locks coordinate matching Store
writers; they do not constrain a privileged filesystem owner or arbitrary SQL.
Linux behavior is not qualified by these macOS fixtures.

Identity credential issuance/revocation, provisioning (including key/Device
retirement) and membership writers acquire their affected write scopes before
entering SQLite and reauthorize inside the transaction. Membership mutations also
cover existing members and any new invitation/join target. Tests hold an actual
reader while real writers queue, prove admission is reserved, then release it and
check the committed credential/key/Device/member state. Snapshot guards include
other members so their retirement waits for snapshot delivery too. The later
[selected writer integration](authority-writers.md) adds ordinary, task, Session
and protected graph transaction paths; that report is the current continuation
of this historical foundation checkpoint. Provider/media callbacks, complete
runtime/recovery and retention/expiry coverage still require integration before
a production peer policy can rely on the complete writer set.

`Store::open_administration` skips runtime adoption, startup recovery and blob
maintenance. It also does not treat a different process epoch as proof that a
worker is abandoned during operation lookup, idempotent admission retry or claim.
Actual expiry and expired worker leases retain their existing recovery semantics.
One-shot server CLI commands use this opening mode. A real child process checks
that an administration lookup/retry cannot steal a live worker and that its
original lease remains usable. Explicit runtime/recovery openings retain their
recovery behavior and require exclusive operational ownership; concurrent server
startup ownership is not qualified here.

Canonical send/history/snapshot work retains its `StoreInvocationAuthority`
inside the blocking Store closure. Canceling the parent async request cannot
release the last guard while an uncancellable blocking effect is pending. The
cancellation fixture blocks at that boundary, aborts the caller, queues a real
credential revocation, then proves the accepted message and its same-ID receipt
settle before revocation. It does not claim cancellation can undo a committed
mutation or prove network delivery.

Retirement watchers, exact signed-key Device admission, all writer paths,
retained subscriptions, HTTP response-body admission, native media cleanup,
production canonical listener installation, full command dispatch and canonical
QUIC/HTTP/SDK parity remain open. No remote host, browser, live provider,
publication or Gate 6 acceptance follows from this fence foundation.
