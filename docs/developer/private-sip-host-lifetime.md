# Original private SIP call host handoff

`Store::private_initial_telephone_phase` reports custodian metadata for the
original private call: quarantine, preparation, captured application, sealed
lifetime, or captured closure. It returns no authority reader and authorizes no
allocation, packet, signal, replay or public readiness. Quarantine retains its
original admission deadline; a prepared attempt keeps its own lease bound.
Captured application requires current execution and live AI provider readiness.
The sealed lifetime follows current physical and provider authority independently
of the ended controller and the child's old preparation deadline.

The host keeps that same owned call through preparation and receiver-revision
changes. It validates the original admission, committed Core resource, exact
ports, preparation/controller binding and original child, canonical Connection,
Core connection and SIP Call-ID. Admission still requires the muted revision 2
observation. Continuing resource metadata tolerates later audience revisions;
it never substitutes for current packet or signal authority. A transient Store
or proof-lock failure retains the original resource while fresh IO guards apply.

The packet policy uses admission only for quarantine control, and denies it for
packets. After captured application it selects the protected current-media reader;
it does not fall back to admission when that read fails. Selection and return
control can resolve the correct private owner, while initial mute and
`PreparePrivateAudio` remain admission-only intents. Return's closure reader
still cannot receive or publish a programme.

After captured application the host installs the real managed SIP DTMF listener
and source registration. Incoming INFO/RFC4733 observations enter the existing
received-signal custody and current directional authorization pipeline. The
listener holds metadata, not an idle reader. Captured closure exits the host's
loop and cleans only its original call; actual absence must still be observed
and recorded before retirement or return completion.

Controlled Store tests and default service compilation are recorded separately
from runtime behavior. This increment does not enable SIP-candidate selection or
qualify actual private media, INFO/RFC4733, return, unrelated private groups or
the complete G6-046 matrix. Those checks remain open. Public commands use the
existing [server HTTP reference](canonical-http.md).
