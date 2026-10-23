# Resumable Session teardown

`Orchestrator::end_session` moves an initiating or active Session to `Ending`
before waiting for cleanup. The first caller fixes its end reason. `Ending`
rejects participant commits, native preparations, new Session views, and both
legacy inbound/outbound binding commits. Ending or retrying a Session does not
change Conversation membership or end another Session.

Before the first suspension, teardown retires physical Session views and pending
native preparations, removes direct subscriptions and reverse Connection lookup,
marks the managed participant controller ended, cancels pending admissions,
aborts its mixer task, and retires live managed Connection generations. Physical
adapter cleanup remains owned by the existing lifecycle supervisor. Uncertain
adapter cleanup continues to quarantine capacity; local Session completion is
not evidence of a provider hangup.

The remaining asynchronous step clears the retained mixer members under the
mixer lock. Cancelling that wait leaves the Session `Ending`, with new media
and admission fenced. A later `end_session` call repeats the idempotent cleanup
and finishes it. Concurrent callers preserve the first reason and produce one
`SessionEnded` event after local cleanup, with a completion timestamp. No new
unowned teardown task or background retry loop is introduced. An already
`Ended` or `Failed` Session retains its existing idempotent behavior.

The same retry rule applies to a cancelled forced Conversation close: the
Conversation remains open, the closing reservation is released, and a later
forced close resumes its `Ending` Sessions before closing the Conversation.
A non-forced close continues to reject while any Session is `Ending`.

`patches/rvoip/gate6-session-cleanup.patch` records this upstream increment.
`scripts/check-gate6-session-cleanup.py` binds qualification to the locked patch
stack and exact source hashes. Deterministic tests hold the actual managed
mixer lock, abort teardown, then retry; they also exercise pending admission
cancellation, live-generation retirement, subscription/reverse-index cleanup,
concurrent end callers, first-reason retention, one completion event, interrupted
force-close, the legacy binding fences, and continuing encoded/decoded audio in
an unrelated Session. The report is
`docs/evidence/gate-6/session-cleanup.json`.

This is an engine teardown correction. Canonical media negotiation, actual
staged-controller/mixer integration, durable logical operation recovery, and
production Connect policy/dispatch installation remain open. See
[peer exchanges](peer-exchange.md) and [native preparation](peer-native.md).
