# Owned runtime startup (G6-037, in progress)

Current schema-31 message dispatch continues in [provider dispatch](provider-dispatch.md).
This startup checkpoint was verified at schema 30. The [local qualification report](../evidence/gate-6/owned-runtime.json)
and [writer inventory](../../contracts/gate6-authority-writers.json) extend the
[operation-worker checkpoint](operation-authority.md). Schema 30 and earlier
candidate databases are refused unchanged. Preserve a verified backup with its
matching binary, then explicitly initialize a new path; no conversion is added.
The marker is a binary protocol boundary: older candidates must not open the new
state and bypass installation ownership.

Store owns the installation protocol. `InstallationOwnership::acquire` obtains
exclusive kernel ownership, refusing a second server or another process. Ordinary
`Store::initialize` / `Store::open` handles retain the owner; handles opened in the
same process share its current instance. `Store::open_for_recovery` acquires a new
exclusive owner, requiring the old runtime to stop. `open_administration` acquires
no owner and performs no connection epoch, video or global worker startup fencing.
Administrative expiry recovery still cannot adopt a live foreign worker.

`open_with_ownership` verifies an explicitly supplied proof for the exact database
and borrows the runtime's lifetime. Server `InstallationLock` wraps this same
Store protocol and retires admissions on shutdown, drop, failed startup and
cancelled startup. It is not a second OS lock. Each admitted SQLite unit retains
an actual owner permit through its work and unlocks SQLite before releasing that
permit. Retirement refuses new admissions, including callers waiting for the
SQLite mutex. An old Store Arc cannot revive a retired runtime or prevent the
next server from starting once admitted work has settled. A pending SQL permit
keeps the kernel lock until completion; retirement never forcibly unlocks it.
This ownership primitive does not yet span all network/body admission or media
cleanup work.

The private adjacent lock name derives from database file identity, without raw
Identity, Device, credential or Conversation IDs. Same-file path/case aliases use
one namespace, while different database extensions do not collide by basename.
Lock shape, owner, permissions, link count and file identity are checked. Unsafe
symlink/hardlink/mode changes and replaced namespaces do not prove ownership.
Kernel process death releases ownership even when the lockfile remains. A stale
file or persisted runtime label is never evidence of an active or stopped owner.
As with the resource authority protocol, live state/lockfile moves, replacement or
unlink by a privileged filesystem owner are unsupported; cooperative matching
binaries must all use the protocol.

Each new owner has a fresh observation epoch, even on a same-process restart.
Handles of that owner share its epoch; restored copies have their own. A copied
durable Connection is not a recovered socket. Connection and transition commit
checks use the Store runtime's current epoch. Administrative handles may recognize
a current local observation label without obtaining ownership; that label grants
no startup authority or physical transport capability. A restored transition
cannot submit from old physical observations, and fresh trusted adapter observations
still cannot release an unresolved operation's reservation or repeat its effect.

Connection epoch startup and current video-binding reconciliation now derive
exact eligible metadata prefixes of at most 1,000 records and 64 merged resource
write scopes, wait without the SQLite mutex, and rederive after admission. The
immediate transaction rechecks eligibility and coverage of every originally
selected ID before mutating any row. New eligible IDs cannot be substituted under
an old proof. Connection changes cover the Conversation and Connection owner
Identity; video Session/binding changes cover the source Conversation. These
metadata changes do not mutate member Identity authorization. Reader policy must
include the Conversation of every referenced Session/Connection.

Startup drains positive batches. Current/terminal/unknown Connection observations
are not invented or revived; interrupted observations lose readiness and advance
generation once. Video bindings become ended and ready Sessions become accepted,
with the existing capacity-checked interruption journal. This protects the current
projection while G6-038 replaces it with the canonical Session model; it adds no
legacy multi-session compatibility. Transactions remain atomic per batch. A later
batch failure may leave earlier committed safe interruption evidence and is not
a successful startup. Retry continues from durable state.

Server ownership, SQLite startup, worker recovery and video reconciliation run in
`spawn_blocking`, outside the async executor. Cancellation does not pretend the
blocking work stopped; its root is retired when the result is discarded. Tests
cover held delivery readers, 1,030 connection/video Conversations, same-process
restart, actual child refusal/death, retired handles and pending SQL ownership,
backup/unknown preservation, prior-schema refusal and startup failure cleanup.
The subprocess entry is ignored standalone and explicitly invoked by required
passing parents, not counted as an unexecuted pass.

G6-037 remains in progress. Complete current reader/held dispatcher scopes, exact
signed-key Device policy, binding retirement watchers, natural expiry and owned
permits through actual network/response-body admission and uncancellable external
work remain open. Existing media regression is not canonical lifecycle cleanup.
Other provider/SMS bulk profiles, production canonical listener/full commands,
QUIC/HTTP/SDK parity, Linux/remote/browser/live-provider lanes, publication and
all Gate 6 acceptance remain unqualified.
