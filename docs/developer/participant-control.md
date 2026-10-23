# Held authority for participant control

The general Core participant engine supports an explicit held-authority path for
admission, individual departure and session-role changes. This supplies a missing
composition seam for G6-020 and canonical Session integration. It does not route
public Session commands into the older two-party Store projection.

`ParticipantPolicy::current_control` receives the exact `ParticipantScope` and an
opaque trusted invocation context. Its default returns `Unsupported`; Core never
falls back to `current` when a held context is supplied. A policy must validate
its domain, current actor/credential, Session/Conversation, target, action,
operation ID and expected revision under the invocation's actual owned guard.
Payload assertions, roles and an unrecognized context cannot supply authority.

The hook returns a `ParticipantControl` containing a real owned
`ParticipantControlLease`, invocation cancellation and one absolute deadline,
capped at 30 seconds. The lease rederives relationship, readiness and attributed
confirmation facts under that same guard at preparation and commit. Its callback
must not reacquire the grant or substitute another credential. Core checks the
original principal, mutation target, current relationship and endpoint/generation
requirements, cancellation and deadlines before effects and at the commit point.
Callback cancellation or expiry is checked again before any presence is published.

Trusted callers use `change_session_participant_with_held` or
`admit_peer_session_connection_with_held`. The latter retains the existing exact
physical peer/device ownership, native preparation, quota and authoritative
adapter-liveness checks; it accepts only admission mutations. Ordinary methods
continue to acquire their normal policy leases.

Preparation and the mixer-lock wait observe caller cancellation, control
cancellation, peer shutdown and the earliest original credential/task expiry,
caller deadline, engine bound and held-control deadline. Task delegation expiry
is independent of a later enclosing credential expiry. A stream wait completing at
the control deadline is classified using the expired invocation before its
readiness result is returned. A saved cancelled context
cannot replay or lend extended credentials to old work. Policies must cancel the
invocation token when the owning handler ends; Core does not cancel a parent
policy signal when a local control call finishes.

Failure, cancellation or future drop removes provisional admission and retires its
candidate route through existing supervised cleanup. The guard remains owned by
the caller through reply delivery and is released when that caller drops it. It
is not stored in mixer members, streams or packet-publication paths. Existing
participant, connection and media guards remain responsible for packet authority;
production guarded mixer adoption is still separate work.

The focused Core tests use a real owned Tokio grant reader with a queued writer.
They exercise admission and subsequent role/departure changes, continuing decoded
PCMU audio on unrelated legs, proof release, unknown/foreign or changed contexts,
preparation cancellation and future drop, original credential/task expiry, mixer
wait cancellation/deadline, fresh final facts, cancellation inside the final
callback, and refusal of cancelled contexts or extended credentials. The broader
participant/session/peer suites retain their existing scope. Their adapters and
policy mapping are controlled fixtures, not live SIP/provider or production Store
policy qualification.

The change is recorded in `patches/rvoip/gate6-participant-held-control.patch` and
the source lock. A fresh prepared dependency tree reproduces the patch; prior
prepared sources are preserved under ignored local scratch. The scoped runner is
`scripts/check-gate6-participant-control.py`.

G6-020 and G6-037 remain in progress. The general durable Session model, current
Connect policy installation, public canonical Session/Connection/Stream handlers,
real offer/answer/ready, managed mixer policy, retained credential rotation and
later coordinator workflows still need implementation and behavior evidence.
No Gate 6 acceptance, remote/provider/browser profile or publication is closed.
