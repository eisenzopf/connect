# Recorded private SIP candidate lifetime

Schema 108 extends the existing recorded private lifetime to the original
registered SIP candidate. The candidate table is role-neutral; native offer,
signing and admission checks remain native-specific. A SIP candidate supplies
its original private call reservation, own private consent, confirmed initial
call effect and committed muted private observation. It never supplies a fake
native admission or borrows an ordinary public SIP call.

Completion still requires the exact once-only audience attempt, captured
application and separately recorded decoded candidate audio. Private readiness
alone cannot establish the lifetime. Captured audience revisions advance the
live SIP readiness from revision two to three while preserving the original
revision-two quarantine observation. The database trigger binds lifetime and
attempt snapshots to the registered root, candidate, original child/Core/SIP
call, Endpoint, own credential and Participant epoch. Matching attempt/lifetime
JSON alone is insufficient; public source kinds and foreign bindings are refused.

Fresh lifetime validation checks the captured native primary/retained peer and
private SIP candidate, current physical credentials, Endpoint/Device, membership
and Session presence, both private receive permissions and reciprocal recipient
ceilings. A SIP candidate must have the distinct `private-initial-sip` kind and
the same consultation root; the existing ordinary SIP-primary/native-candidate
profile keeps its own typed checks. Physical metadata validation does not recurse
through this lifetime, so it cannot accidentally borrow the ended execution
credential.

The captured lifetime can survive expiration or revocation of a separate
controller credential, including when recording already observed completion.
Current physical withdrawal still refuses lifetime validation. Historical
completion replay preserves the recorded result and cannot recreate current
authority. Backup preserves the capture; a restarted runtime cannot claim the
old call or lifetime. Stores from earlier schemas are refused at a new open;
there is no automatic upgrade, downgrade or compatibility path.

This is the Store recording and metadata validation component of G6-046. Its
controlled observers are not actual SIP/Core private audience evidence. The
runtime coordinator still selects native candidates, and private SIP media and
signal readers still refuse ordinary/public access. Pending/established packet
intervals, outside-pair handling, host lifetime handoff and actual
private audio/INFO/RFC4733/return/cleanup remain required. No clause or task closes
from these component checks. The existing consultation and operation methods in
the [server HTTP reference](canonical-http.md) remain the client interface.

The [recorded SIP candidate return](private-sip-candidate-return.md) now extends
Store return admission, closure control and exact captured retirement at schema
109; actual audience restoration remains unqualified.
