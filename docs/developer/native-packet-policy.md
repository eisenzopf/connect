# Current packet policy under owned file custody

`Store::try_native_media_read` captures current native source/target facts and
acquires their complete current authority footprint. The SQL version seal rejects
concurrent mutation, and the resulting reader owns the file fences through the
bounded local media action.

`Store::try_native_media_read_under(target, source, held)` supports protected
composition within an active packet's existing reader. It checks that the reader
is current and belongs to this exact Store namespace and runtime owner. It then
derives the requested pair's current facts using the same SQL-versioned metadata
rules and requires coverage of every current scope. Foreign, incomplete or
expired custody is refused without acquiring replacement readers.

Only the file interval is shared. The holder's participant, credentials,
permissions, audience or stream binding do not become the recipient's authority.
The pair still checks its own current source/target credentials, signing keys,
Devices, Endpoints, membership epochs, Connection generations, recipient ceilings
and subscription permission. Its expiry cannot exceed either its current facts
or the original holder's expiry. The SQL mutex is released before returning.

The caller must retain the exact frame's route/subscription guards, cancellation
and original 200 ms age bound. This API performs no media IO and supplies no
permission for queued work to reuse an earlier authorization. Supported writers
remain fenced until the last owned reader drops; composition can finish under
that same interval without entering a new reader behind a waiting writer.

The mixer now associates each consumed segment with its own still-owned source
lease. Its synchronous recipient check can reuse that lease's opaque file
context. The outgoing queue retains no owned file reader or permission grant. After dequeue,
guarded egress obtains current source authority, then checks the exact recipient
under its file custody. Store namespace, scope coverage and both current physical
bindings remain checked. Other policy implementations retain independent fresh
admission; data-command custody is separate.

The [Store component report](../evidence/gate-6/auxiliary-native-packet-reader.json)
records current-policy and owned-fence behavior. The
[integration checkpoint](../evidence/gate-6/auxiliary-native-packet-custody.json)
records controlled Core tests, the completed exact host build and actual runtime
results, including unresolved telephone movement and 50-peer failures. The canonical HTTP wire contract is unchanged; see its
[OpenAPI reference](../../contracts/openapi-canonical-http.json).


The native custodian lifetime poll can use a positive unchanged SQL-versioned
expiry observation without acquiring file readers. A missing hint, mutation,
clock edge or expired source returns to complete current admission before any
retirement decision. This observation only keeps the original custodian alive;
it cannot authorize media or a control operation.

`Store::try_native_session_role_hint` similarly exposes only an unchanged role
string. A consumer may skip a disposable projection when Core already has that
role. Applying a different role still requires the complete current role read,
source set and original mutation fences. Neither hint retains a reader across
idle time or changes the original credential expiry. The
[observation report](../evidence/gate-6/auxiliary-native-observation-hints.json)
records controlled current-authority, SQL mutation and expiry checks and labels
its pending host integration separately.


Retained audio already carries the exact original consumer and scene stamps.
Core captures its current port, stream revision and credential snapshot under a
short model lock, releases that lock while obtaining fresh policy/file authority,
then immediately reacquires and revalidates the captured state. A busy probe
releases every acquired reader before the original bounded retry waits. Even a
pause/resume restoring the same final permission changes the revision and refuses
the older callback. Existing route, consumer and scene generations still gate IO.
Initial/current-gate capture, data and held control paths keep their existing
locking; this optimization cannot recapture a newer audience for an old packet.
The [packet admission report](../evidence/gate-6/auxiliary-native-packet-admission.json)
records verification and pending runtime scope separately.


Concurrent copies of the same original audio packet can share an already-active
source file interval. The source policy exposes an opaque weak reference, and
Core clones one weak-metadata slot with that packet's mixer contributions. Every
new input packet gets a separate slot. A queued copy cannot keep a reader alive,
resurrect expired custody, or borrow another packet's interval. When no same-packet
reader remains active, admission must obtain a fresh source read.

`Store::try_native_media_source_read_under` checks the exact original runtime
Connection, generation, participant and membership epoch before deriving current
SQL-versioned facts and complete scope coverage. Recipient authority is still
checked separately. Current revocation, readiness, stream/route/consumer/scene
fences, cancellation and the original 200 ms bound continue to apply. Source and
recipient reads cannot extend the original holder's expiry. Other policies can
retain independent admission by omitting the weak-reference opt-in. The
[same-packet custody report](../evidence/gate-6/auxiliary-same-packet-source-custody.json)
separates controlled verification from actual host and capacity evidence.


An owned media interval may expire before the current credential.
`MediaAuthorityLease::expires_at` exposes that additional immutable bound without
reacquiring policy or file custody. Core caps packet IO, local neutral output
and discard-only preparation input at the earliest of this expiry, credential
expiry and the original media deadline. An expired interval refuses before IO;
a longer-lived refreshed credential cannot extend an older retained interval.
The [owned expiry report](../evidence/gate-6/auxiliary-packet-owned-expiry.json)
records controlled pending-IO, no-poll, release and decoded-recovery evidence
separately from the current host and capacity checks.


An already-owned complete file interval needs no file admission between SQL
capture and sealing. That path retains one short SQL mutex interval while it
checks current facts, exact SQL/clock stamps and complete scope coverage. It
releases SQL before returning to IO. Fresh file admission retains the separate
capture/release/admit/reacquire/seal path, so file work never runs with SQL held.
External commits can still happen despite a process-local SQL mutex; they must
continue to fail the version seal. The
[owned SQL report](../evidence/gate-6/auxiliary-packet-owned-sql.json) records
mutation, queued-writer, clock-change and release evidence separately from the
remaining actual capacity checks.


The canonical native adapter now collects ready single-source audio metadata into
finite epochs: 1 ms collection, at most 64 jobs per epoch and 4096 pending jobs.
Exact-original copies use the synchronous Core batch; every recipient still
checks current authority and the original 200 ms deadline. The queue and its
execution wait hold metadata alone. The port owns its receipt and the epoch join,
so cancellation or a failed dispatch cannot report cleanup before the actual
sender finishes. Admission refusals may retry unchanged metadata; a submitted
transport error is terminal. The [native scheduler component report](../evidence/gate-6/auxiliary-native-ready-audio.json)
records bounded queue and actual QUIC cleanup/recovery evidence. The fixed optimized
full-fifty rehearsal now passes both signing algorithms with complete validated
captures. All twelve affected production methods and22 captured lifecycle
envelopes also pass on the same unchanged executable. This internal scheduling
change leaves the [canonical HTTP reference](../../contracts/openapi-canonical-http.json)
unchanged.
