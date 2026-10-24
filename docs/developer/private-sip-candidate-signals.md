# Private SIP candidate signal readers

A typed private initial SIP origin can now obtain a signal reader through
`Store::try_sip_signal_read` and a directional native/SIP pair through
`Store::try_sip_native_signal_read`. The initial native/SIP pair helper uses the
same protected constructor. Ordinary `try_initial_telephone_signal_read` and
public initial media readers continue to reject a private origin.

Preparation and muted connection readiness do not authorize signals. A pending
consultation needs captured audience application and current original execution
authority. Its sealed physical lifetime survives the old controller ending, but
requires the original current credentials, native keys, Endpoint, Device, member
epochs, private partner bindings and recipient ceilings. An applied return closes
signal access before physical call cleanup.

The reader acquires the complete private context, including the independent SIP
route owner, and checks coverage again inside the owned interval. Both directions
require the named native and initial SIP Endpoint permissions. Initial SIP uses
its Endpoint's `dtmf.send`/`dtmf.receive`; established SIP retains its route-owned
directional permission model. Current route dialing and recipient policy remain
required. Codec preferences and media offer/subscription permissions do not grant
or withdraw named signal rights. Signal readers cannot become media readers.

Pair matching recognizes either an ordinary SIP primary with its original native
consultant, or a private SIP candidate with its original native primary. Both
physical private floors must be reciprocal under the same root and runtime.
Held remotes and public siblings cannot borrow the pair. Received INFO/RFC4733
custody freezes only that exact native recipient and rechecks current receive
permission before recording a physical delivery attempt.

The current Store tests cover these rules. They construct received observations;
they do not prove actual private SIP-candidate INFO/RFC4733 delivery. Host lifetime
handoff, packet/control contexts, runtime SIP-candidate selection and actual
private audience/media/DTMF/return qualification remain required. Existing
SIP-primary socket regressions are recorded separately from these new Store
checks. Public commands remain documented in the [server HTTP reference](canonical-http.md).
