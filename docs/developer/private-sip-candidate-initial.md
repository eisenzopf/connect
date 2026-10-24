# Private SIP candidate call custody

`Store::prepare_private_initial_telephone` derives an initial-call intent from
an opaque [SIP reservation](private-sip-candidate-reservation.md). It reuses the
reserved canonical Connection ID, original consultation, controller credential,
verified Endpoint/Device, member epoch and route policy. The child retains the
two original private members as its audience. It creates no public invitation
receipt, second Connection, native signing identity or physical call.

`InitialTelephoneSource` now has common `conversation_id` and `session_id`
fields and an explicit `InitialTelephoneParent`. `Invitation` carries the real
completed invitation request and record. `Consultation` carries the original
consultation record and reserved Connection ID. Public initial-call preparation
continues to require its exact successful invitation and original target. A
private candidate does not fabricate an invitation to satisfy that check.

The common once-only allocation/signaling custody preserves the original child
operation, canonical Connection, actual Core runtime ID and planned SIP Call-ID.
Retries cannot allocate another call or change either physical identifier.
Private allocation and packet intervals fence the complete original execution
audience, endpoint owner and independent route owner; they recheck current
authority each time and retain no idle readers.

Schema 105 introduced the immutable parent kind, original private audience/epochs,
Endpoint/Device, controller and policy in the child row. This is a fresh
pre-alpha schema, not an in-place upgrade. Older stores are refused without
modification and require a new state path.

[Schema 106 private answer consent](private-sip-candidate-answer.md) now records
an actual candidate answer under its own current Endpoint authority without
public Session acceptance or codec changes. Its immutable typed receipt grants
no media readiness; the ordinary media projection continues to refuse it.

A pending child allocation contributes to the parent's unsettled custody.
Preflight/initialization failure cannot silently release it. Cancellation and
stale-worker recovery preserve an unknown outcome and refuse subsequent IO.
Recovery and terminal-history purge use the same unsettled view, so a pending
child cannot lose its parent history. Exact unsubmitted abandonment or captured
same-runtime absence settles the original child; permission revocation cannot
prevent that recording. Cleanup does not invent a known remote submission
outcome or authorize redial.

The public host activation and public own-acceptance/media paths refuse private
candidates. If a private answer reaches the public recording API, its captured
fact is retained without a public `session.accept`, ready observation or media
grant. The Connection and child outcome remain unknown until owned cleanup.
The [Core private outbound path](private-sip-candidate-core.md) now supplies the
engine admission API. Installing it with guarded private readiness is the next host boundary;
this refusal is not a completed private-call implementation.

The [component report](../evidence/gate-6/auxiliary-private-sip-candidate-initial.json)
records controlled Store behavior and current compilation/integration checks.
Actual muted Core admission, typed SIP-candidate media/DTMF selection, return and
physical cleanup still require implementation and live qualification. No new
HTTP method is exposed; the [server HTTP reference](canonical-http.md) retains
the existing public methods.
