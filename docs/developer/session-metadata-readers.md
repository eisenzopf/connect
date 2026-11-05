# Session metadata polling connections

The installed application still has unresolved media/control and browser
scheduling failures. A local eight-second native sample of the production
source39 synthetic server found substantial inclusive stacks in
`Store::session_reconnect_page`, `Store::session_lifecycle_hint_page` and SQLite
preparation. Both workers poll every 100 ms. Each page opened a new read-only
connection, causing SQLite to parse the database schema repeatedly.

Store now owns two dedicated read-only connections, one per metadata poller.
They are separate from the foreground command mutex and packet observers. A
concurrent call to the same busy reader refuses immediately rather than queuing
behind it; the existing worker retains its cursor and observes again later.
The existing 50 ms SQLite busy timeout remains in force.

Each page still creates and drops its own deferred transaction. Connections
retain no snapshot, permission, result page or authorization decision between
calls. SQLite refreshes schema/data from committed WAL state at the next read.
Reconnect decisions still perform their existing current-state checks and
protected mutations; lifecycle templates still require current authorization at
delivery. Polling cadence, page limits, loss/grace deadlines, journals, media
timestamps and quality thresholds are unchanged. The two connections close with
the Store and add a fixed two-descriptor allocation per Store instance.

The same local optimized empty-page probe measured 204 ms before and 7 ms after
for 30 pairs of original public Store calls. These are single-host component
measurements, including first use, without a timing assertion. They establish
reduced polling overhead; they do not establish the cause of the installed app
failure or qualify any media route. The probe also checks that subsequent schema
commits remain readable and that completed polls do not pin the WAL.

Reader and lifecycle regressions cover read-only enforcement, independent
committed snapshots, immediate refusal while a reader is busy, new commits after
an old page ends, and warm original pollers observing loss and terminal facts.
The component app CI lane runs these before packaging the changed runtime.
Actual installed app, native media and full transport qualification remain
separate required evidence.

A source-bound production server comparison subsequently starts both original
servers from byte-identical copies of one stopped synthetic database. The same
eight-second observer records 3270 ms process CPU over 8228 ms before and 230 ms
over 8137 ms after, a 92.9% reduction in normalized idle CPU rate in this one
pair. SQL preparation inclusive stack counts fall from 285 to 15. All 4812 Git
source byte/executable-mode identities and 4094 unchanged prepared byte/mode
inputs are checked; Rust/manifest/lock differences contain only the six Store
implementation/test files. Raw stack curation is independently recomputed. Both
servers join without force or remaining group members. Process counters and
joins are original controller observations, not reconstructed process logs.
See `docs/evidence/gate-6/session-readers-local-native-idle-observed.json`. This
confirms lower idle polling overhead; sustained media and app CI remain required.


Active-media sampling also found repeated operation-row SQL preparation. Those
fixed row queries now reuse bounded compiled statements (capacity 32 on the
foreground Store connection and four read-only packet observers), with every call rebinding arguments and executing
against current data. Tests alternate operation IDs and verify external commits,
local rollback, schema changes and completed-read WAL release. A single optimized
local 200-read probe measured 80 ms before and 7 ms after; this is component evidence,
not a hosted media result.

Packet metadata now keeps up to four original counterparties per SQL observer,
with 16 paired entries per live origin across four observers. Previously an entry
for another source replaced the prior source even inside one unchanged version.
The actual four-source/four-observer counterexample derived 48 times; current code
derives 16 times for those 16 unique pairs. Unary metadata remains one entry per
observer. Dead source identities and superseded observer versions are removed;
FIFO capacity eviction only causes fresh derivation, never excludes a peer.
Every lookup still compares SQL/data/schema/clock/source identity and acquires a
fresh complete authority interval. No row result, permission or IO grant is
cached. Current-writer refusal and commit/source invalidation remain explicit.
See `docs/evidence/gate-6/packet-source-operation-sql.json` for 67 current cases.


The original change set capacity32 only on command SQL. Native data invocation
also resolves operation/source facts on the independent read-only observers;
those now use the same shared32statement bound. The actual observer200rowprobe
measures13ms before and9ms after in one local pair, with no timing assertion.
Its parameter/data/schema/read-only/WAL checks and75current Store functions pass.
This small component result does not establish why the8a reliable-data scene
exceeded its original200msmetadata budget. See
`docs/evidence/gate-6/observer-operation-sql-cache.json` and the retained negative
`docs/evidence/gate-6/native-av-source-paired-sql-negative-8a.json`.


The G6-058 browser diagnostic v182 distinguishes full SQL-observer saturation
from failure to reacquire the original observer after file admission. The latter
accounted for2,297 of2,536 observer refusals in that run. A current memo miss now
rederives facts under the freshly acquired old-scope reader and one SQL interval.
Only complete coverage of the resulting current scopes permits sealing; a
changed scope set releases that interval before normal fresh admission. The
original SQL/schema/clock/source checks remain mandatory. Only facts returned by
a successful seal may enter the memo, including when an optimistic capture falls
back to a different observer's memo entry.

The prior implementation fails the new external-commit regression by admitting
the same scopes twice. The candidate passes that case and12 related local ARM
checks for snapshot invalidation, source changes, writer exclusion, original
expiry and failed-read cleanup. See
[the before/after evidence](../evidence/gate-6/covered-scope-snapshot-fix-v184.json).
This is component verification; live media improvement remains unproven.
