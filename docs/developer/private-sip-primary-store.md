# Private SIP primary: protected Store authority

The protected Store scene API can bind a SIP primary to its separately admitted
native consultant and a retained remote. This is a Store component: service
SIP-primary dispatch, private Core controls and actual private SIP media/DTMF
qualification remain pending. The existing service caller explicitly converts
its native selections to the typed representation.

`try_native_consultation_scene_read` takes private
`ConsultationMediaOrigin` values. Each origin retains its actual transport facts.
Native snapshots keep native signing/device fields; initial and committed SIP
snapshots keep the actual call, generation, member epoch, Endpoint and route.
Selection freezes those snapshots while the complete current scene interval is
held. An origin or frozen value retains no reader and cannot execute IO.

A selected SIP slot must be the requested primary. Its native counterpart must
match the exact registered consultant Connection, original operation,
controller credential/digest, primary and runtime. Participant identity alone
is insufficient: a public physical sibling of the consultant cannot inherit
private grants. Planning checks the existing registered candidate floor; the
primary floor begins with the prepared audience attempt. An existing public SIP
call cannot stand in for a separately admitted SIP candidate.

The bounded read exposes separate `sip_receive(native, sip)` and
`sip_send(sip, native)` pairs. Each carries the actual SIP read and actual native
endpoint view. `media(native, Some(sip))` returns no native projection for a SIP
source. Private direction grants require current receive permission and
reciprocal recipient ceilings for the phone Participant's identity, including
its independent route-owner policy. Held remotes and unrelated physical slots
receive no private pair. Existing private pairs must retain their own current
context and original registration.

Preparation still records one exact plan before an attempt can begin. A captured
outcome updates only the original SIP generation and receiver revision once.
Established private-context validation checks current typed bindings, original
audience, permissions and expiry independently of the ended execution
credential. Revocation can refuse the next read; historical metadata cannot
revive a call or lend another Connection its role.

The [component report](../evidence/gate-6/auxiliary-private-sip-primary-store.json)
records 178 distinct controlled Store cases, 193 executions and current service
compilation. Seven new cases cover initial endpoint/route separation, complete
private sets, typed directions, current permissions/ceilings, held-remote
exclusion, idle-reader release, captured private lifetime and the public-sibling
regression. Captured Store observations are controlled fixture evidence, not
network audio or SDK delivery. Failed intermediate attempts remain recorded.

Actual private SIP runtime/DTMF, SIP-candidate admission, remaining authority and
retirement compositions, and the complete current-source G6-046 matrix are still
required. The [retained-call profiles](retained-sip-authority.md) keep their
separately recorded scope. The
[canonical HTTP reference](../../contracts/openapi-canonical-http.json) and wire
contract are unchanged by this protected Rust API increment.
