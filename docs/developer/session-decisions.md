# Pending invitation decisions

The installed G6-037 slice routes `session.reject` and `session.cancel` through
Connect's durable service. It does not invoke a stock Core conversation handler.
The current invited appearance may reject; the current creator or explicitly
authorized manager may cancel an invitation before any invitee accepts. Once
any invitee accepts, `session.cancel` conflicts even if other invitees are still
pending; use `session.end` to clear the Session. Rejected post-acceptance
cancellation preserves accepted and pending appearances and the topology revision. See [general Session control](general-sessions.md). An optional `by` is a checked Conversation Participant ID, independent
of the authenticated Identity and enrolled Device. No request field grants
authority. The independent `uctp:session` floor and current `session.reject` or
`session.cancel` permission are required.

The SDK's `CanonicalClient::decide_session_invitation` uses a saved
`session_decisions::Request`. The request retains the logical operation ID,
Conversation/Session IDs, decision, Participant assertion, optional expected
topology revision, reason code (0–65535) and reason (at most 256 UTF-8 bytes,
without control characters). It validates signed replies and their correlation,
resource IDs, completed decision state, revision and exact original reasons.
There is no automatic HTTP fallback.
The SDK's earlier application operation adapter and the earlier native
`connect.request` application profile explicitly refuse this new command family.

The explicit current HTTP client sends the same canonical `session.reject` or
`session.cancel` envelope to `/v2/commands`. Its shared typed helper validates the
same result as the native client. The [current canonical HTTP
reference](../../contracts/openapi-canonical-http.json) documents envelopes,
correlation, budgets and errors. The generic application route refuses the older
`{"op":"session_decision",...}` body instead of providing a compatibility path.

Both paths use the existing operation ledger and source authority fences. A
signed decision reuses its exact owned proof; Binding-only, changed-intent,
foreign or expired proofs cannot acquire a replacement grant. The mutation
checks pending invitation state and the optional topology revision inside the
receipt transaction. A committed decision advances topology once and emits
the ordinary scoped journal fact. Exact replay returns the original decision
under fresh current authority, including after its revision advances. Changed
intent conflicts; stale authorized decisions retain a failed receipt while
rolling back Session changes. Spoofed Participant assertions have no receipt.

`operation.updated` plus ACK reports the durable decision. It does not claim
Session acceptance/readiness or media termination. The journal is authoritative;
notifications remain hints. No other invitation or Connection is changed.

Verification is recorded in `docs/evidence/gate-6/session-decisions.json`.
Tests arrange invitations with the existing trusted fixture API; they do not
qualify the later general Session creation model. The completed shared dispatch
checkpoint is recorded in `docs/evidence/gate-6/canonical-http.json`. G6-038 retains the
general multi-participant Session projection and orchestration.

Native callers can additionally use
`CanonicalClient::cancel_session_invitation(original_request_id, request)` to
assert ownership of their original invitation wire ID. This keeps the same
ordinary logical cancellation receipt and current authority checks. Same-Device
siblings, wrong Session references and superseded references refuse before
mutation. Reconnect by replaying the original logical invitation for a new
reference, then replay the same cancellation operation. A configured human
consent requirement suspends the original command through normal step-up;
references never upgrade assurance or permission. Unsigned HTTP rejects these
physical references and continues to accept ordinary logical cancellation.
