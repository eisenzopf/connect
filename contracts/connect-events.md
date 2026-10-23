# Connect snapshots, events and observations

Gate 6 target; G6-008.

## Durable facts

Each journal event has an opaque event ID, installation cursor, Conversation ID,
optional Session/Connection/Participant IDs, operation correlation, kind,
authoritative resource revision, source/provenance, audience and observed time.
Only the journal cursor defines order. Times support diagnostics, not ordering.
A single accepted mutation and its local facts commit atomically.

`conversation.get` returns an authorized snapshot with its high watermark.
`conversation.subscribe` accepts `after` and a bounded live lease. It returns a
snapshot/resync boundary followed by `conversation.event` items newer than the
observed cursor, without a snapshot/live race. A client drains pagination before
acting on a complete view. Event gaps can arise from filtering or other
conversations; gaps are not private-event counts.

Clients persist their last actually processed cursor. Reconnect reauthenticates
and resumes from it; duplicate events may be observed and are idempotently
handled. Retention loss returns an explicit resync-required result and snapshot
reference, never an apparently complete empty page. Reads use fresh request IDs;
operation retry cannot be used to pin an indefinitely stale authorized snapshot.

## Session start

Initial activation requires a current ready public Connection for every accepted
Participant. Multiple Connections for one Participant cannot cover another
Participant. The native source is an independently observed direction-appropriate
flow under its original installed lifetime; provisional readiness JSON and
negotiation alone cannot satisfy it. Private candidates, retired generations,
foreign runtime rows and sources without current authority do not supply public
readiness. SIP additionally requires the actual owned connected observation and
current independent Endpoint/route authority. Pending invitees do not count as
accepted Participants. The bounded lifecycle worker rechecks accepted Sessions
after membership/public-scope changes without another media submission.

Schema93 atomically commits the last required readiness observation, the Session's
Active phase and server-observed activation time, an immutable `session_started`
fact with the observed ready public Connections, and one journal row per admitted present Participant. A storage
failure rolls that transaction back; metadata retry uses the existing observed
flow without submitting more media. Once started, the separate last-Connection
loss and permitted-negotiation grace rules govern the Session; a later invitation
does not rewrite its original start.

The transport name for actual telephone interop is `sip-interop`. Global selected
capabilities retain only common descriptors. A `connections` extension records
exact selected descriptors per Connection when they differ; a transcoded codec
pair is not reported as a common endpoint format. The same current actual native
and SIP origins govern this metadata snapshot; rows cannot reconstruct IO origins.

`session.started` carries CID/SID, original `started_at`, `participants_present`,
`active_connections` with Connection ID, Participant ID and transport, and actual
`negotiated_capabilities`. An unsolicited native hint has no Connection scope,
operation ID or reply correlation. Each current authenticated recipient route
is independently authorized immediately before writing. That physical recipient
route needs no media Connection or subscription of its own; another current
Device may supply readiness for the same accepted Participant. A narrow recipient
ceiling cannot expose the snapshot.
Finite HTTP/native journal catch-up also emits the original typed hint before
its cursor-bearing event, with the subscription's reply correlation.

Hints are bounded best effort. Later invitations, replay and hint loss do not
rewrite the original fact. Current source access and membership epochs govern
recovery. The native first-start implementation does not yet qualify Participant
joined/left, last-Connection loss/grace, browser, SIP or remote hosting profiles.

## Bounded delivery

Default page limit is 100, maximum 500 and 512 KiB encoded response. Default live
lease is 30 seconds and ends no later than credential expiry. A subscriber queue
holds at most 256 event references; notification pressure never blocks media or
the journal. A slow consumer receives a resync boundary or disconnect and recovers
from the store. Revocation stops future delivery and reauthorizes queued items.

## Transient observations

`session.observation` has a typed kind: transcript, speech or action-request.
It includes participant, Connection/generation, provider source, correlation,
turn/fragment ID where available, observed time and completeness. Transcript
fields distinguish interim/final/incomplete; speech reports started/stopped;
action-request names a declared supported action and provider tool-call ID.

The server binds provenance from the actual provider/media route, not from a
model-selected session or recipient. A provider lacking speech or tool events
advertises that limitation. Missing speech events are not proof of silence.
Only policy-selected bounded observations become durable context; they retain
their original audience and attribution.

A separate worker may wait for observed speech completion and validate an action
proposal. It owns scheduling, prompts, approval decisions and business state.
Connect supplies observations, durable operations, context and authorized
execution. A model's requested action is not permission or completion.

## Participant connected presence

Schema94 separates the accepted Session appearance from verified public connected
presence. A first public ready observation appends `session_participant_joined`
with its original role/name/kind/identity, membership epoch and via-Connection.
More Connections for that same appearance do not create more joined events.
Logical pending/replacement Connections preserve that appearance while settlement
is in flight. Explicit Session leave and membership removal, or confirmation that
the last Connection has ended, append an immutable left fact. Re-entry after a
left fact creates a new joined fact on the same SID; it cannot rewrite old facts.

Start, joined and left use one bounded source-ordered native hint worker. Every
actual write reacquires current recipient authority; hints hold no idle media
reader and confer no IO rights. Historical typed hints are available through
finite HTTP/native catch-up and precede their original cursor-bearing events.
Role/name changes do not rewrite prior joined bodies. Full reconnect grace,
rapid-loss/restart aggregation and private promotion require separate evidence.

Participant facts retain their own stable `fact_id`. Every native or catch-up
hint has a fresh envelope ID. Native delivery authorizes the exact internal fact
reference and CID/SID before the guarded transport assigns the final signed wire
ID; wire IDs are not source grants or journal deduplication keys.
Journal recipient PID/epoch and owner-only operation correlation are supplied
at insertion, including when departure removes the subject's current membership.
Later admission cannot rewrite or revive an old recipient epoch.
