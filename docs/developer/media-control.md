# Held current media control

Canonical handlers already retain a current authority guard through execution
and their response. Asking `MediaPolicy::current` to reacquire the same store
lock during staged control can fail or deadlock behind a queued writer.
`MediaPolicy::current_control` is the explicit composition hook for this case.
Its default is Unsupported; there is no fallback to another grant read.

The hook receives the exact Core scope, requested engine intent and the trusted
opaque invocation context. The installed policy must validate its own domain,
current actor and credential properties, resource mapping, action/revision and
scoped grants under the context's still-held guard. It must not await or reacquire
that guard's store lock. Wire assertions, roles, capabilities and an unrecognized
context type cannot supply authority. `MediaControlIntent` describes initialization,
a change's desired/replacement streams and expected revision, pause/resume, or
an exact logical stream end. Permission for one action does not imply another.

The policy returns an owned `MediaControl`: its real authority lease, an
invocation cancellation token and an absolute deadline, capped at 30 seconds.
Core rechecks that lease's cancellation/deadline and its actual principal,
membership, endpoint, Connection generation and model revision at each control
decision. Waits use the earliest of caller, control, credential, task-delegation and engine bounds. The original task-delegation
expiry is checked independently of a still-held current grant; it also fences
new packet admission, forwarding and dequeued delivery.
Cancellation, expiry or a dropped future retires provisional candidates while
preserving existing media and cumulative binding accounting. This is authority
for the current control invocation; it is never installed in media ports as a
publication grant.

`StreamController::with_held_control`, `change_with_held`,
`set_paused_with_held` and `end_stream_with_held` use this hook. They retain the
same controller, normal current media policy, allocator and history. Ordinary
publication, forwarding and dequeued sends still use `MediaPolicy::current`.
They do not inherit the control context. Normal standalone control APIs remain
available when no application authority guard is already held.

A preparing Connection can stage its first candidate without packet authority.
The lease can rederive readiness under its same grant through `control_owner`;
commit still requires Active state. See [initial media admission](initial-media-admission.md)
for the state boundary, rollback behavior and scoped verification.

`CanonicalNativeAdapter::configure_rtp_with_held` composes this constructor with
the sealed native allocation and frozen-origin checks. It cannot reset or replace
an installed controller. `AuthorizedEnvelope::invocation_cancellation` supplies a
child of the actual driven handler's lifetime. Returning from the handler, EOF,
expiry or abort cancels that child even if trusted server code saved its context.
Cancelling the child cannot cancel the parent invocation.

The scoped runner is `scripts/check-gate6-media-control.py`; its source-bound
report is `docs/evidence/gate-6/media-control.json`. Core tests exercise a real
held Tokio read guard with a queued writer through construction/change and
pause/resume/end, fail-closed context/domain/scope/grant checks, time bounds,
credential/task expiry, cancellation/drop cleanup, and fresh per-packet authorization after the
control guard is released. Actual authenticated local TLS QUIC/WebTransport
handlers configure and change the native controller using the held guard, verify
writer progress through the signed response, reject saved contexts after return,
and apply fresh hold/resume and packet policy checks. Their codec/readiness and
store projections are controlled; ACKs report the observed engine revision only.

This increment supplies the engine/dispatch composition seam. It does not install
the Connect durable store policy, canonical offer/answer/ready negotiation,
endpoint confirmation, managed mixer/retained subscription handlers, or logical
operation recovery. Equivalent held-authority participant composition is documented in
[participant control](participant-control.md). Controllers retain their original
credential bound until an explicit checked
[credential rebind](media-credential-rebind.md). Production automatic refresh and
step-up installation remain open; a replacement credential cannot be lent to old
control work or old queued media. No Gate 6 acceptance or complete canonical
media negotiation is inferred from these fixtures.
