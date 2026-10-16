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
