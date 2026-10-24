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
context. The outgoing queue stores no context or permission reader. After dequeue,
guarded egress acquires a new current source lease, then checks the exact recipient
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
