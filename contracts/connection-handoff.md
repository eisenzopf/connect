# Connection-preserving handoff (G6-022)

The managed rvoip Session prepares and commits an authorized three-party audio
handoff. It retains the remote Connection, main Session and admitted incoming
media bindings. Participant handoff requires distinct outgoing, incoming and
remote Participants. Replacing an endpoint for one Participant is a separate
operation. These engine observations are local facts, not durable Connect
operation receipts or public presence.

## Preparation and acceptance

An incoming Connection is already admitted in a closed private audience with the
outgoing participant. Core requires successfully decoded input and a completed
guarded output send on that Connection after the private context began. Failed
or merely queued sends do not count. The evidence pins the actual controller,
credential incarnation, port, consumer and subscription generations. A silent
interval does not invalidate a healthy current generation; a gate, credential,
consumer, resource or subscription change invalidates its old evidence.

Planning produces metadata only. It neither retires a healthy route nor grants
permission. An offer, accepted API command, codec/resource readiness or a caller
`ready` boolean cannot commit the handoff.

G6-040 adds `plan_staged_prepared_audio_handoff` for a target whose actual duplex
IO occurred in an independent one-member closed audience. Guarded local endpoint
audio can prove the output side without delivering the old private participant's
speech. The optional previous audience must contain the outgoing connection;
an occupied outgoing private audience cannot be omitted. Both contexts are exact
scene-issued handles and are consumed by the same role/media commit. This API
does not initialize a provider or prove that its opaque memory is fresh; those
remain independently enforced host responsibilities.

An unselected old private connection retains a closed quarantine and an empty
receive programme after the previous audience is consumed. Neither a broad
Participant subscription nor reuse of its old restoration handle can publish
that private context into the new pair. The independent target preparation and
queued local prompts also become stale at commit. Ordinary public and original
private-pair handoff remain explicit preparation profiles.

Acceptance is supplied by the host's current policy, outside the media engine.
`MediaControlIntent::Handoff` is separate from ordinary audience changes. The
managed entry point also requires `ParticipantPolicy::current_handoff_control`
for the actual operator principal, complete managed plan and participant
revision. Missing policy returns Unsupported before effects. Current actor,
outgoing and incoming Memberships and explicit media permissions come from the
held grant. Role labels do not grant permissions. The desired incoming role is
explicit host intent; prompts and business decisions remain outside Core.

## One commit

The request pins the private audience, remote and admitted Connections, audience
revision, per-controller resource revisions, participant revision, operation ID
and outgoing disposition. Current authority stays stable under the actual held
domain grant through bounded preparation; callbacks cannot reacquire that domain
or reenter engine state. Handles and plans keep no authorization reader.

Core freezes relevant controllers, stages receiver programmes and every outgoing
participant Connection's speaking role, and drains admitted IO. Under the managed
Session, lifecycle, relationship and scene fences it validates a role/departure
candidate and every media candidate before publishing any of them. The original
private group and queued private output cannot cross the new audience.

The retained remote and incoming Connection become the speaking pair. The
outgoing Participant either receives the call as an authorized Observer on its
admitted Connections, or leaves. Observer source admission is disabled, so a
simple unmute cannot restore the old speaking path. Leave fences and retires the
outgoing media only after successful publication; other participants and retained
physical bindings remain intact. New observer admissions cannot become speakers
merely because their independent media permission is true.

The participant model commits both role/departure changes in one revision with
one correlated local observation. No later role update is used to disguise a
separate media commit. Cancellation, deadline, current denial, stale state,
competing acceptance, changed ownership or remote termination before commit
preserve the previous usable bridge or expose the actual ended state.

An active audience can be explicitly retargeted through
`plan_retarget_staged_audio_audience` with its exact current handle and a new
whole-operation grant. This permits later private consultation and another
handoff in the same retained call. It does not restore an old programme or grant;
old audience/controller plans remain stale after the new commit.

## Recovery and cleanup

The result distinguishes a known local bridge/model commit from pending or
unconfirmed cleanup. Adapter teardown is owned and supervised. Local media
cleanup can be observed separately and retried on the same original resources;
it does not establish remote/provider acknowledgment or undo external effects.
An uncertain cleanup cannot make the new bridge roll back or redial.

`replay_staged_audio_handoff` returns the exact historical local role/bridge
observation only after fresh current operator authority. The participant model
pins actor/target identities and membership epochs, refuses changed intent or
cross-family operation-ID reuse, and preserves original correlation metadata.
This history does not claim the original bridge is still active after a later
operation. Re-executing an old physical plan conflicts; it does not perform a
second switch. Connect's durable journal and reconciliation remain authoritative
for the complete public operation and restart recovery.

Native resource confirmation carries forward only through the actual sealed
`AudioAudienceCommit` proof on unchanged installed bindings. Retiring Connections
cannot manufacture another confirmation. The managed native fixture uses real
signed authentication, distinct identities, TLS QUIC/WebTransport, actual codec
IO and controlled negotiation/domain storage; it does not claim a live provider
or complete public workflow installation.

## Qualification

`scripts/check-gate6-connection-handoff.py` records executed cases, source digests,
logs and test executable digests. The original G6-022 qualification is recorded
in the Gate 6 implementation plan. Subsequent G6-040 extensions require their
own current-source behavior evidence. Linux x86_64, remote and
browser acceptance, durable service/HTTP/SDK integration, named provider
rehearsals under G6-093 and publication remain assigned Gate 6 work.
