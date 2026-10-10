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

Contract revision 3 adds owner-authorized runner coordination: `Claim`,
`ClaimGet` and `ReleaseClaim`. A fresh claim requires no prior revision;
renewal/reclaim uses the exact retained revision. Leases last at most 30 seconds,
within task and credential lifetime, and at most 16 may be active. Matching
request replay returns the original lease without extending it. Reclaim is
denied once the step has a durable dispatched, uncertain or terminal effect;
read/reconcile that effect instead. Worker IDs are coordination labels, not
credentials. Narrow model invocation grants cannot acquire runner authority.

`Invoke.wait_for_reply` explicitly marks a message question. It defaults to
false and is omitted when false, preserving previous serialized inputs. Only
Send may wait or carry `reply_to`. That reference must be a delivered initiating
event or a reply assigned to the calling task. An assistant answering another
task's verified question uses its own current send grant; the recipient task is
derived from the question, never borrowed as authority. An accepted question can
have a succeeded communication effect while its tool result stays pending until
the specific attributed answer exists. Other replies cannot satisfy that wait.

Effects retain the original logical request ID. The store derives a separate
wire receipt ID from task, step and logical ID so two tasks cannot alias an
account-level receipt. The operation receipt, local mutation, effect intent,
authority reference and journal events commit atomically. Known business
rejections retain an optional effect `error`; missing `error` in older results
defaults to none. Infrastructure failure rolls back rather than accepting a
partial operation. Queued provider jobs stay pending; unknown remote outcomes
stay uncertain without redispatch. Legacy effects without protected dispatch
metadata remain read-only and cannot be promoted by assuming missing jobs passed.

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

Recall traverses the existing recipient conversation journal oldest first, with
at most 100 source entries and a 32 KiB serialized result including metadata and
cursor. `omitted` counts eligible entries after the input cursor not returned
in that page. An individually oversized body returns an unavailable source with
an explicit budget note and advances the cursor; it never stalls catch-up or
silently truncates evidence. Missing sources also remain explicit. Messages
have no timestamp in the existing schema, so `created_at` is null with a
timestamp-not-recorded note rather than a fabricated date. If a missing source
also lost its attribution, `actor_id` is empty.

Session and artifact bodies project current metadata rather than stale journal
snapshots. Deleted, expired, missing or invalid artifact blobs have no body;
recall never follows symlinks or exports file bytes/paths. Metadata is conversation
context under history/context-view authority. Artifact byte retrieval separately
requires the current task, invocation, actor and artifact-get authority. Cursors
bind installation, identity, task and recall resource; every continuation rechecks
the current grant, credential and conversation access.

Admission reserves one terminal journal entry and one mutation receipt for each
nonterminal task within the existing installation limits. Ordinary operations,
nonterminal progress and new admission cannot spend this room. A terminal transition
updates state and consumes its released reservation in the same transaction.
Elapsed time does not free a reservation before the expiry event commits. A
write failure or a configured limit lowered below retained usage returns an
explicit error without acknowledging cancellation; recovery retries use the
same logical ID. The reserve protects terminal control records, not an unlimited
number of later reconciliation observations or a filesystem write guarantee.

A matching terminal lifecycle transition receipt remains inspectable under the
current owner's task-transition scope and conversation access, as cancellation
receipts already are. Revoked reader credentials fail. This inspection neither
reopens the task nor revives its grants; a new transition and every effect replay
still enforce continuation authority. Creation receipts and effect receipts do
not gain an exception from this terminal-control rule.

Contract errors use existing Connect `{code,message}` shapes. Unknown profiles:
`unsupported-profile`; malformed input: `invalid-request`; missing current
credential: `unauthorized`; disallowed operation/resource/binding: `forbidden`;
stale or illegal state: `task-state-conflict` / `effect-state-conflict`; mismatched
logical intent: `request-conflict`; capacity: `capacity-exceeded`; storage failure:
`storage-unavailable`. Pending and uncertain tool results never claim completion.
Lease conflicts use `task-claim-conflict`; a step that requires effect inspection
instead of reclamation uses `effect-reconciliation-required`. A known provider
failure uses `provider-failed` and retains its accepted local outcome separately.

Typed fixtures prove admission and exhaustive state transitions. Runtime denial,
crash/recovery, provider and actual host evidence are separate required items.
