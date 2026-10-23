# Retained media credential rebinding

The Core stream controller can replace its authenticated credential through
`rebind_credential_with_held`. It retains the same policy, physical route,
logical model, ports, transport IDs, local handles and cumulative open budget.
The credential revision is independent of the stream revision; a rebind requires
both expected revisions and advances only the credential revision.

The installed `MediaPolicy::current_control` must explicitly authorize
`MediaControlIntent::RebindCredential` under the actual current invocation guard.
Its scope carries the newly authenticated principal. Its intent carries the
previous principal and both revisions. The policy must bind the exact credential
incarnations, domain, actor, resources and action, including current grants and
retirement state. The default refuses this path. Principal metadata, a claimed
expiry, or a wire assertion cannot establish that proof.

Core refuses a change of issuer/tenant/subject owner. Current policy must resolve
the same mapped Identity, Participant/membership epoch, endpoint/device and exact
Connection generation. It rechecks current relationship and preparation facts at
both ends of the operation. A fresh credential does not confer media grants or
make an unready Connection active. Closed controllers and terminal routes cannot
be revived. The caller retains its guard through reply delivery; ports never
retain the rebind control proof.

Each ordinary or held media control captures its original credential before
acquiring the mutation slot. The slot serializes rebind with negotiation,
pause/resume and stream end. Rebind conflicts with an existing pending change.
A command that captured an older credential before a completed rebind also
conflicts. Its callbacks, preparation and expiry cannot borrow the new principal.

Rebind waits for admitted sends on retained retired ports, then quiesces each
active port and drains its admitted local sends. A terminal callback cannot hide
still-running local I/O from this boundary. Caller/control cancellation, owning-route shutdown, the absolute control
deadline and the new principal's earliest credential/task expiry bound that wait.
Failure or dropping the future restores temporary delivery admission. Successful
commit advances the ports' private delivery generations before installing the
new credential. Older ingress packets and queued outbound frames then fail their
original tickets; new frames use fresh current policy. Paused/ended streams,
quarantined cleanup and previously consumed port-open history remain intact.

Local media sends also retain their original source and destination credential
bounds. Their timeout is the earliest credential/task expiry or the existing
200 ms send bound. A held grant cannot extend an already admitted send past its
original credential lifetime. Expiry is checked again after policy callbacks
before polling the send, including when a callback outlasts the source credential.

`Membership::with_authenticated_identity` gives trusted current resolvers a new
credential-bound view of the same active membership. It preserves lifecycle,
revision, epoch, role and mapped identity and refuses a different owner or an
inactive membership. It supplies no grant and performs no durable mutation.

`CanonicalNativeAdapter::rebind_media_credential_with_held` additionally requires
the exact physical `PeerAdmission` owner of the existing native route. A second
physical peer with the same Identity/device cannot borrow that route. The adapter
reuses its installed controller and allocator; it does not allocate another
Connection or infer media readiness from authentication.

The scoped runner is `scripts/check-gate6-media-credential-rebind.py`. Core tests
exercise real held Tokio grant readers and queued writers, stable IDs/history,
decoded audio after original expiry, old queued media refusal, pending control
isolation, admitted-send drain, cancellation/deadline/future drop, final policy
facts, task expiry, terminal routes and membership views. Signed local TLS QUIC
and WebTransport fixtures authenticate a refresh, invoke the trusted native SPI
under the next command's current guard, retain quota/controller/handles, and
decode RTP after the original credential expires. Another case refuses a foreign
physical peer, progresses behind a queued grant writer through the signed reply,
then enforces fresh packet denial.

Those G6-020 wire handlers and policy/readiness resolvers are controlled fixtures;
they are not automatic Connect installation evidence.

## Connect production refresh (G6-046)

The production `StorePeerAuthority` now receives a sealed verified refresh after
successor authentication, same-owner/assurance checks and successor dispatch/reply
preparation. It releases Binding-only readers before fresh union admission across
both enrolled credentials and the exact original media/audience scopes. The
runtime selects only the actual physical `PeerAdmission` and transport binding;
another peer with the same Identity cannot be selected by Identity alone.

Schema86 retains immutable original admissions and records distinct credential
epochs with prepared/applied/refused/uncertain outcomes. A started or uncertain
rotation blocks fresh old-origin reads. Only a captured actual Core revision
selects the successor credential in the current admission projection. Pending
original stream controls refuse refresh before submission so their custody is
not stranded. A bounded owned worker rotates each original controller once,
records the returned revisions and swaps immutable policy entries. Old Arcs keep
old origins. Captured results remain factual custody after recording failure;
they confer no authority to retry Core.

The adapter's sealed controller result creates fresh data, DTMF, lifecycle,
capability and server-reconciliation registrations for the successor dispatcher.
It neither fabricates a signed original offer nor retags queued old payloads.
Same-credential refresh only renews dispatch metadata. Possible submission or
successor-publication uncertainty retires the physical peer.

[Current evidence](../evidence/gate-6/native-credential-refresh.json) includes actual
signed native QUIC for EdDSA and ES256, DTMF in both directions, unchanged epochs
on repeated same-credential refresh, and independent FFmpeg decoding of PCMU after
the original credential expires. Generation, endpoint, Participant, membership
epoch, stream revision and actual media handles remain stable.

Task/owned step-up, granular successor scope changes, production fault/recovery
injection, other transports and exact61 auxiliary-clause qualification remain
open. Local macOS arm64 qualification does not qualify Linux, remote/provider/
browser release profiles, full G6-046, Gate 6 acceptance or publication.
