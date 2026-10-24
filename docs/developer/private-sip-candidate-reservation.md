# Private SIP candidate reservation

`Store::reserve_sip_consultation_leg` reserves a fresh canonical Connection for
the original consultation before a SIP call can be allocated. The opaque
`SipConsultationReservation` contains metadata only. It owns no authority reader
and provides no signaling, media, allocation or readiness capability.

The reservation pins the original controller credential, command digest,
runtime, primary Connection, two private Participants and membership epochs,
verified SIP/telephone Endpoint revision and Device, own endpoint credential,
and independent route credential and policy. Both credentials must currently
authorize the actual private identities. The route owner is not substituted for
the phone Participant. The Session must accept audio only.

Reservation uses a Conversation writer and current identity fences, then
revalidates in an immediate transaction. It consumes one bounded preparing slot
without creating a call, offer, effect, operation or journal event. An exact
retry returns the original Connection ID. A changed policy cannot replace it.
Another native or SIP candidate cannot claim the same consultation. Public or
private native offers cannot borrow the SIP Connection ID.

Schema 104 introduced immutable `sip_consultation_reservations` and a shared
`consultation_candidate_bindings` metadata view. Snapshot visibility and the
private observation floor include SIP reservations. Only the original private
members at their original epochs can see candidate metadata; a later membership
does not inherit it. An unused reservation is never a current physical source.
The existing primary floor timing remains tied to the actual prepared attempt.
The current [typed call-parent boundary](private-sip-candidate-initial.md) uses
schema 105. Older stores are refused without modification; use a fresh state
path for this pre-alpha schema.

`validate_sip_consultation_reservation` rechecks current execution, endpoint,
route, member and Session authority and the exact unused canonical binding. It
does not return an IO grant. `retire_unused_sip_consultation_reservation` can
release the exact unused slot after controller revocation without renewing that
credential. It refuses observed runtime/readiness or a linked initial call,
retains the original row, and advances retirement once. A retry cannot resurrect
the Connection.

The [component report](../evidence/gate-6/auxiliary-private-sip-candidate-reservation.json)
records controlled Store verification and its failed intermediate attempts.
The typed initial-call child now preserves this reservation through controlled
allocation/signaling custody. Actual private SIP call admission, muted Core quarantine, typed SIP-candidate
selection, media/DTMF, return and cleanup remain to be implemented and verified.
The [SIP-primary runtime](private-sip-primary-store.md) has a separately recorded
scope. This protected Rust API adds no public HTTP method; the
[server HTTP reference](canonical-http.md) is unchanged.
