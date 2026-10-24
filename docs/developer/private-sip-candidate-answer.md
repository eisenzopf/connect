# Private SIP candidate answer consent

The installed SIP adapter captures an actual answer for the original allocated
runtime Connection and SIP Call-ID. Call
`Store::record_private_initial_telephone_answer` with that opaque capture for a
private candidate. The Store records the immutable answer and confirmed effect
even if authority was withdrawn after signaling. Capturing an answer grants no
permission to allocate, signal again, publish media or declare readiness.

The separate consent step rechecks the original consultation and reserved
Connection, controller execution, own Endpoint credential and `session.accept`,
independent route authority, verified Endpoint/Device, original membership
epochs and private audience. It accepts the candidate's observed negotiated
codec for this Connection. It does not execute a public `session.accept`, change
public Session presence/capabilities, allocate another Connection, or fabricate
a native signing identity.

Schema 106 adds immutable `initial_telephone_private_acceptances` rows. Each
binds the original child and consultation, Endpoint credential/ID/revision,
Participant/epoch, observed codec and private audience. The common answer marks
its acceptance kind explicitly: `private-connection` references the child
operation and this exact private receipt; `public-session` still requires the
real completed own Session acceptance receipt. SQL refuses accepted answer
insertion before consent, wrong receipt kinds and changes to either immutable
consent. The private receipt participates in snapshot digests. This remains a
fresh pre-alpha schema; existing older stores are refused rather than migrated.

Exact private replay retains the original consent identity and rechecks current
authority before returning the captured media origin. Changed observations
conflict. A refused or public-fallback answer cannot be promoted later by
replay; its captured fact and cleanup custody remain. The public recording API
always refuses to expose private consent as public media, including after a
successful private acceptance.

An accepted private answer is not media readiness. The current ordinary media
projection continues to refuse it. The host still requires its separate
guarded private readiness path and actual muted
[Core private admission](private-sip-candidate-core.md), followed by typed
candidate lifetime/media/DTMF, return and physical cleanup. Controlled Store
tests qualify consent and authority behavior only; they establish no SIP wire
delivery or complete G6-046 clause.

The [call custody guide](private-sip-candidate-initial.md) describes allocation
and recovery. No new public HTTP method is introduced; the
[server HTTP reference](canonical-http.md) retains the existing methods.
