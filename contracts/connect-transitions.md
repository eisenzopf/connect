# Connect live transition contract

Gate 6 target; G6-006. All transitions use the operation contract and current
audience/authority rules. Handoff, consultation and device replacement are
operations on the six nouns, not new primary communication nouns.

## State and readiness

Legal operation states: accepted, preparing, ready, committing, completed,
cancelled, failed, interrupted, unknown. Admission failure returns a typed error
without accepting work. Legal state edges are defined in the operation schema;
terminal results never regress after duplicate or late callbacks.

Mutations of an existing membership or media topology require its expected
revision. Only one conflicting topology transition can hold its execution lease.
Readiness requires permitted participant/endpoint, negotiated compatible media,
verified ingress and egress readiness, and any configured endpoint confirmation.
SDP generation, a SIP answer, WebRTC signaling state or API acceptance alone is
insufficient. A readiness observation must match the current connection generation.

Before commit, cancellation restores the retained usable route and retires only
prepared resources. At/after commit, cancellation cannot promise rollback; report
the committed result and use a new authorized transition. If actual commit is
uncertain, hold conflicting new actions until reconciliation.

## Consultation

`session.consult` names the retained remote connection, explicit private
participant set and desired hold treatment. Connect prepares the private leg and
context, then switches only the authorized audience. Conversation, main Session
and remote Connection IDs remain stable. A separate provider AI session may be
required; it retains the logical AI Participant identity and receives only the
private projection. Never reuse a provider context that leaks consultation into
the public leg.

Cancel/return restores the earlier authorized speaking relationship. If the
remote hangs up, end or explicitly settle the affected engagement and cancel
pending media. Holding a connection does not fabricate a continuing remote call.

## Participant handoff

`session.handoff` names source and target participants, retained remote connection,
candidate connection if already prepared, expected revision and outgoing
disposition (`leave` or permitted `observe`). The coordinator verifies policy,
target admission and media readiness before committing speaking responsibility.
It changes only the intended role/speaking bindings and reports retained, admitted
and retired connection IDs. A model action request is merely intent.

`role_scope` defaults to `conversation`; require an additional
`expected_conversation_revision` and atomically update the Conversation role
defaults and selected Session's responsibility. Explicit `session` scope changes
only that live engagement. Other active Sessions retain their admitted role
assignments. `leave` retires the source's Session presence, not its Conversation
membership or historical access. Ordinary message arrival advances journal order,
not these topology/membership revisions.

Both AI-to-human and human-to-AI use the same operation. The outgoing observer
has no automatic access to private streams or historical content. Failed target
admission leaves the existing usable route intact.

## Same-participant endpoint replacement

`connection.replace` targets the current participant's Connection and a provisioned
replacement Endpoint or already prepared candidate Connection. It retains the
Conversation, main Session and Participant; a new Connection is expected. The
original remote connection remains unchanged. Browser/phone is one configuration
of this generic operation.

An endpoint confirmation policy may require an attributed DTMF 1 after answer
and media readiness, with a 45-second default callback deadline. Voicemail,
incorrect/no confirmation, no-answer, cancellation or timeout cannot steal the
speaking route. Client-supplied phone numbers do not bypass endpoint provisioning.
The old route remains until commit; it is then explicitly retired. A late event
from its old generation cannot reactivate it.

## Identity and stream invariants

| Action | Stable | Replaced / updated |
|---|---|---|
| Consultation | Conversation, main Session, remote Connection | Private audience and provider consulting leg |
| Handoff | Conversation, main Session, remote Connection | Active participant responsibility and intended media binding |
| Device replace | Conversation, main Session, Participant, remote Connection | Participant endpoint Connection and its physical streams |
| QUIC migration | All logical objects and Connection | Physical network path only |
| Media reconnect in grace | Conversation, Session, Participant | Lost Connection and physical stream bindings |
| Session end | Conversation and authorized history | Live resources terminate; outcome record emitted |

Logical stream references can retain application continuity through an explicit
replacement mapping; transport stream IDs and RTP bindings are never falsely
reported unchanged. Physical-peer datagram handles are not reused.

## Failures and concurrency

Test before/after each prepare, readiness, commit and retirement boundary:
remote hangup, candidate failure, revocation, lease expiry, worker loss, server
restart, duplicate/reordered callbacks and competing controllers. A state conflict
is not latest-wins. Only current authority can reconcile an unknown external state;
a human attestation is recorded as an attestation, not a remote termination command.
