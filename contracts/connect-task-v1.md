# Connect task application extension v1

The frozen typed inputs/results are `connect_protocol::tasks`. Required profile:
`connect-task/1`, an additive application operation inside `connect-control/1`.
This is Connect task policy carried over UCTP, not an upstream UCTP revision.
The control operation is wired in G4-013; capabilities advertise it before use.
Old control commands retain their behavior and authority. Unknown profile/action
or claimed owner/binding fields fail before admission.

Task states: ready → running → waiting → ready; running → completed. Any
nonterminal state may become cancelled, failed or expired. Terminal states never
reopen. Mutations use one stable logical request ID and expected revision where
state-dependent. Matching replay needs current authority before receipt lookup.

Effect states: prepared → dispatched/cancelled; dispatched → succeeded/failed/
uncertain; uncertain → succeeded/failed from attributed reconciliation only.
An expired lease cannot turn possible dispatch back into prepared work.

Task input cannot supply owner, credential or provider authority. Creation derives
those from authenticated state and validates existing conversation membership,
concrete recipient/operation subsets and expiry. Provider call/chat correlation
is registered by trusted composition, never inferred from a model's headers.
Grant issuance is outside the model-facing command enum; five-minute grants
cannot refresh themselves. Protected host adapters may refresh only through
current backing authority and unchanged task scope.

Limits and the executable acceptance mapping are in
[decision 009](../docs/decisions/009-task-runtime.md). Task events have stable
source/actor attribution. Native `Reply` takes conversation, question reference, body and logical ID under
the sender’s current `send` authority. It cannot choose task ownership. The store
derives a task only from a verified question delivered to that participant and
commits message, reply/inbox and lifecycle atomically before ACK. Terminal tasks
retain unassigned incoming evidence rather than reopening.

Reply assignment is explicit and auditable; ambiguous
replies remain unassigned. A task cursor binds installation, principal, task and
journal sequence. Recall has a 32 KiB budget, explicit omissions and source
provenance. A missing timestamp is explicit for legacy sources; no fabricated
metadata. Retrieval text cannot change grants or routes.

Contract errors use existing Connect `{code,message}` shapes. Unknown profiles:
`unsupported-profile`; malformed input: `invalid-request`; missing current
credential: `unauthorized`; disallowed operation/resource/binding: `forbidden`;
stale or illegal state: `task-state-conflict` / `effect-state-conflict`; mismatched
logical intent: `request-conflict`; capacity: `capacity-exceeded`; storage failure:
`storage-unavailable`. Pending and uncertain tool results never claim completion.

Typed fixtures prove admission and exhaustive state transitions. Runtime denial,
crash/recovery, provider and actual host evidence are separate required items.
