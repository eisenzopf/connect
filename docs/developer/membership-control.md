# Membership control (G6-037, in progress)

Fresh schema 34 connects canonical `conversation.participant.add`, `.update` and
`.remove` to the ordinary control engine and durable operation ledger. HTTP,
public Rust, TypeScript and Python helpers, and named identity MCP tools expose
all ten membership actions. This extends the historical
[conversation-control checkpoint](conversation-control.md); its schema-33 report
is retained unchanged. The current HTTP contract is
[OpenAPI membership control](../../contracts/openapi-membership.json).

A membership request contains `operation_id`, `conversation_id`,
`expected_revision` and a closed tagged `change`. Actions are `add`, `invite`,
`join`, `decline`, `cancel_invitation`, `leave`, `remove`, `update_role`,
`share_history` and `revoke_history_share`. The public helpers are
`change_membership`/`changeMembership`, `membership_snapshot`/`membershipSnapshot`
and `membership_invitations`/`membershipInvitations`. Python exports TypedDicts;
TypeScript exports the tagged action and outcome types. SDK mutation helpers
require the exact completed `local-mutation` receipt and status reference, with
the same logical ID and Conversation. They never retry a mutation automatically.

`add` is explicit admission by a current authorized manager. It matches the frozen
canonical Add body, which contains Identity, kind and role, without invitation
expiry or acceptance fields. It is distinct from the optional invitation/target
acceptance workflow; it does not mint a credential or confer permission through a
role label. Existing [membership and audience policy](membership.md) governs
content, history sharing, pending invitation cancellation and logical presence.
All mutation actions require their named scope, current actor authority, same
tenant and applicable recipient ceilings. Target kind is checked against the
current live Identity. Source readers and affected Identity/Conversation writers
are acquired before SQL and checked again under the transaction.

An ordinary member's receipt captures its Participant ID and original membership
epoch before effects. Leaving and readmission cannot revive old operations.
Nonmember administrator and invitation-target origins are explicit receipt
variants with current authorization checks. Successful join captures the newly
admitted epoch. The original completed self-leave/self-remove receipt can still
be recovered while inactive in that same epoch; readmission ends this exception.
A remove reason is retained in the participant view and journal. Readmission
clears it and increments the membership epoch without restoring old content or
history-share grants.

Exact replay retains its logical receipt and projects current authorized state.
Changed normalized intent under the same owner/ID conflicts. Effects, revision,
journal facts and receipt commit together. Authorized business conflicts retain a
failed operation; infrastructure failures roll back and reserve no ID. Request
status and `operation.get` use the same current projection and original actor
binding. Opaque IDs share the protocol's ASCII letters/digits/`._~-` domain.

Signed canonical admission binds every typed action field, resource, credential,
enrolled Device/key and installation. The proof remains owned through SQL and
reply submission. A queued retirement writer cannot force reader reentry beneath
an already admitted mutation. Source expiry during an SQL wait rolls back all
effects. Success returns the actual stored `operation.updated` record followed
by a correlated `conversation.participant.changed` fact. A missing required wire
revision is refused by envelope validation before trusted correlation. Lost reply
recovery queries the original operation; credential rotation retains current
original permission and the same stable actor epoch.

Membership snapshot requires current membership and `conversation.get`, filters
participants by current recipient ceiling and caps the result at 128. Personal
invitations require `conversation.participant.add` and include only the target's
pending, unexpired entries from live issuers in open same-tenant Conversations.
Composed Store callers use `membership_snapshot_with_authority` with an already
sufficient owned guard; it rechecks current credential, membership and coverage
inside SQL without reacquiring below a queued writer.
The encrypted opaque cursor binds Identity, credential, tenant, normalized scope,
purpose and a fixed sequence high-water. It expires within 30 seconds or the
credential lifetime; rotation, tampering or a restored earlier high-water refuses
it. Pages cap count at 100, encoded entries at 384 KiB and authority scope budget.
The cursor does not advance past unreturned entries or expose private gap counts.
These are current views, not immutable retained snapshots.

Identity MCP tools use closed flat arguments and inject the action on the server:
`connect_participant_add`, `_invite`, `_join`, `_decline`, `_cancel_invitation`,
`_leave`, `_remove`, `_update_role`, plus `connect_history_share`,
`connect_history_share_revoke`, `connect_membership_snapshot` and
`connect_membership_invitations`. Discovery requires each named capability.
Caller-supplied actor, credential and action override fields are refused.

The [membership-control evidence](../evidence/gate-6/membership-control.json)
records the exact selected local candidate and test inventory. Its runtime lanes
cover all ten real MCP actions with HTTP/native/SDK replay, signed QUIC lost-reply
recovery and rotation, proof substitution, queued retirement, expiry behind SQL,
atomic storage failure, concurrent same-ID admission, restart, encrypted invitation
pagination and old schema refusal. Historical reports and intermediate failures
are retained separately. They are not current-source passes.

Production signed native listener/client configuration, remaining canonical
commands and subscriptions, complete per-underlying-write HTTP response custody,
physical media retirement, remote/browser/provider profiles, independent
conformance and Gate 6 acceptance remain open. Logical membership removal alone
does not certify physical provider hangup or media teardown. This checkpoint does
not qualify publication.
