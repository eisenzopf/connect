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
