# Canonical peer authentication lifecycle

This guide records the peer authentication increment; the final authentication audit records current reusable task qualification. `rvoip_uctp::v2::auth_peer::CanonicalPeer` owns the
server side of the envelope-v2 authentication handshake and the resulting peer
binding. One actor drives one instance for a secure reliable physical peer. It
uses the v2 authenticated dispatcher and never hands commands to a v1 coordinator.
The local QUIC fixture negotiates `uctp/2`; automatic production adapter and
Connect service installation remain open.

`auth.hello` must offer `connect/2` and an installed authentication method. The
challenge advertises only the intersection with installed methods, Ed25519
signing and the requirement for signed proofs/controls. A failed profile or
method negotiation performs no credential validation or application work.
The server creates fresh random challenge IDs/nonces. `auth.response` must name
that exact challenge in `in_reply_to`, contain no resource/operation scope and
provide a validated credential and a signature from its subject-owned key.
The signed unpredictable challenge ID binds the proof to this physical peer's
pending challenge. The ten-second authentication deadline starts when the peer
is created and does not reset on input; eight failed pre-auth attempts close it.

An unscoped handshake returns an authenticated Identity, Device binding, finite
expiry, assurance, opaque session token and selected profile. It creates no
Conversation membership or Session presence. The former mandatory
`auth.session.participant_id` contradicted the conversation-scoped Participant
model. The corrected schema makes it optional and requires `cid` when supplied.
Generated Rust and the shared vectors carry the same correction. A client Device
ID is a claim: the installed policy must check ownership before admission.

`PeerAuthority` receives the complete `ValidatedBearer`, including token ID and
issue time, together with the Device, mapped Identity and original envelope.
Identity mapping is resolved before acquiring an owned guard, then must remain
current under that guard. The bound application policy retains this credential
metadata for every later invocation; it cannot silently borrow another token's
authority because its subject matches. Implementations must cover credential
and signing-key revocation, device/mapping ownership and resource permissions.
The new runtime fixtures provide a controlled policy; Connect's store-backed
installation remains G6-037.

A refresh is signed and authorized under the prior credential. The replacement
credential must retain subject, issuer, tenant, mapped Identity and intrinsic
assurance provenance. Identified credential kinds must match; task-scoped
assurance retains its Identity, task and scopes; user-authorized assurance retains
its Identity, delegating user and scopes. A refresh cannot perform an implicit
assurance upgrade or downgrade. The policy explicitly approves current old/new
credentials and effective grant-scope changes under one owned guard. Step-up
requires its separate correlated transition and is not implemented by refresh.

Successful refresh creates a new dispatch incarnation, rotates the session token
and retires old waiters. It does not lend new credential authority to an old
invocation. Failed, unavailable, timed-out or cancelled validation preserves the
prior binding while it remains valid. A caller's shorter request deadline is
separate from peer credential expiry. Retrying after uncertainty needs a fresh
signed envelope; ordinary logical mutations retain their operation ID.

Keepalive requires a current signature, current credential authority and this
incarnation's exact session token, compared with the auth library's constant-time
bounded token helper. It does not extend expiry or implement reconnect/resumption.
The old token stops working after refresh. Client logout is signed and closes
this peer's lifetime. Idle credential/task expiry emits a signed `auth.bye` and
retires the lifetime even when no inbound frame arrives.

`Sig9421Signer` owns an Ed25519 PKCS#8 key and signs all original envelope fields
through the existing strict JCS implementation. Its diagnostics redact keys and
key IDs. Server replies receive fresh wire IDs/timestamps and signatures; validated
requests receive correlated replies/problems without credential or payload echo.
Malformed JSON is not used to construct correlation metadata.

`PeerOutput` retains current authority through bounded writer admission. Direct
`handle` integrations must retain it until their write completes. `run` owns the
length-prefixed reader/writer with the shared 1 MiB cap, bounds writes and drains
on EOF, expiry, cancellation or writer failure. Its child lifetime is available
for sibling signaling/media pumps; closing the peer leaves a shared parent
endpoint alive. Physical adapters must install that lifetime and the selected
profile deliberately. A signed reply is an observation, not a later mutation
grant; application operations and media activation still need current authority.

Run `python3 scripts/check-gate6-auth-peer.py` for source-bound local qualification.
The checks include a real pinned-TLS/QUIC stream and controlled credential/policy
fixtures, plus existing local auth and RTP regressions. They do not qualify pending
step-up challenge validation, blocked-request recovery, assurance-change events,
production adapter/Connect policy installation, durable restart replay, resumption,
SDK/browser interoperability, remote hosting or live providers. G6-026 is open.

The [correlated step-up implementation](step-up.md) now owns bounded pending
challenges, one-use recovery of verified blocked mutations and scoped verified
assurance-change replies. Its evidence is separate from this guide's original
handshake report. Production installation and the final G6-026 audit remain open.

The [authentication recovery implementation](auth-recovery.md) adds the retained
physical-peer replay fence, explicit signature problem taxonomy and dual scoped
step-up failure outcomes, including unknown interrupted execution. Its report
supersedes the earlier partial recovery boundaries for these source changes.

The [live-peer reachability implementation](peer-reachability.md) now adds staged
trusted route registration, bounded keepalive renewal, current revocation
lifetimes and guarded actor-backed outbound submission. Its separate evidence
does not qualify production profile installation or durable endpoint routing.


Pre-authentication `auth.hello` and `auth.response` now have a separate eight-entry,
non-evicting ID fence. IDs are consumed before credential validation can await.
A failed or cancelled attempt requires a fresh wire ID correlated to the same
current challenge; duplicates do not repeat credential work. The initial hello
counts toward the eight entries. Exhaustion fails closed, with the existing
absolute ten-second authentication deadline and failed-attempt limit unchanged.
Authenticated signature replay retention remains separate. Challenge capabilities
advertise the pre-auth capacity and canonical default command scope names.
Current qualification is linked from [`secure-profile.md`](secure-profile.md).


Canonical bindings now retain one physical resource context through credential
refresh and step-up. The context is carried privately in authorized envelopes;
owner drop retires only that peer's actual managed core routes, retaining quota
through uncertain cleanup. See [`peer-admission.md`](peer-admission.md) for the
32-associated-Session/64-Connection managed admission boundary and remaining
integration work. [Pending Session views](peer-sessions.md) extend that quota
to zero-Connection creation and authorized attachment, retaining the logical
Session across physical EOF. Invitation delivery/acceptance and native
allocation remain open.


The [retirement lifecycle](assurance-retirement.md) now requires a policy-supplied
current credential-incarnation lifetime and publishes a signed terminal
assurance-change before goodbye on expiry or factual authority retirement.
It preserves old/candidate/successor isolation and interrupts candidate step-up
execution if that candidate's authority retires. Its report qualifies these
changes; historical reports retain their original source scope.


[EdDSA/ES256 signing and the final auth scope audit](es256.md) now advertise the
implemented algorithms and existing identified assurance floor. That report and
the task-level audit cover current reusable authentication semantics; the earlier
sections/reports retain their original incremental qualification scope.
