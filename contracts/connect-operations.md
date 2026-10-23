# Connect logical operations and external effects

Gate 6 target; G6-007. A transport envelope and a durable mutation have different
identities. This distinction resolves the baseline idempotency/signature conflict.

The native server may also generate a stable operation ID for materializing an
already committed Participant receive intent. Its internal `stream.reconcile`
record retains the explicit subscription record ID, exact plan, original native
source and one-attempt effect custody. It is not a wire command or another
subscription grant. Existing current-authorized operation status and cancellation
apply. Captured native outcomes retry durable recording without repeating Core
or retaining old authorization readers; unknown outcomes are never resubmitted.

Authorized `operation.get` status and durable transition admission replies include
`record_id`, the Store's stable record identifier for that operation. Native
control status replies expose the same identifier. It is distinct from the
caller-owned `operation_id`; it conveys no permission or completion by itself.
A handoff target may reference its own signed native offer record as `policy_ref`
only while that physical peer's current named handoff policy permits the route.

Current-authorized status and transition replies also retain the optional
durable `problem` object. A definitive native handoff refusal before any native
attempt records `failed`, an attributable bounded reason and no media effect or
success result. Repeating that intent returns the same refusal. This path cannot
settle prepared or possibly submitted native work; uncertainty retains its
original custody and requires reconciliation. Temporary missing decoded audio
readiness is not classified as a definitive policy refusal.

## Admission and identity

A mutating command has a caller-generated stable `operation_id`. Its ownership
key is installation/tenant plus authenticated stable principal plus operation ID.
Credential rotation does not create a new owner; changed effective authority is
checked every time. Actor or tenant fields supplied by a client are not authority.

The service validates and normalizes the typed command, then hashes its canonical
business intent (type, resource IDs, expected revision and payload). Transport
ID/timestamp/signature and credentials are excluded. The canonical byte form is
UTF-8 [JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785);
duplicate keys and nonfinite numbers are
invalid. The server computes the authoritative digest. Matching identity/digest
returns the existing authorized operation. Different intent under the same key
returns conflict, including after a completed/unknown result.

An accepted receipt means durable admission, not completed media or provider
delivery. It includes operation ID, current state, resource scope and a status
reference. The journal atomically records local state and intended effects before
dispatch. Query/cancel/reconcile obey current access checks and return only visible
results; a cached success cannot restore revoked access.

## Signed transport retry

Each wire retry has a fresh envelope `id` and current `ts`/signature while retaining
the same `operation_id` and business intent. The signature covers all present
envelope fields except signature itself, including operation ID and revision.
Signature replay protection rejects reused/stale signed envelopes independently
of operation deduplication. The client first queries the accepted operation after
uncertain transport failure rather than generating a new mutation ID.

## Allowed transitions

| From | To |
|---|---|
| accepted | preparing, cancelled, failed, interrupted |
| preparing | ready, committing, cancelled, failed, interrupted, unknown |
| ready | committing, cancelled, failed, interrupted |
| committing | completed, failed, interrupted, unknown |
| interrupted | preparing (proven safe), completed, cancelled, failed, unknown |
| unknown | completed, cancelled, failed after attributed reconciliation |
| completed / cancelled / failed | same terminal state only |

`failed` means a known failure. A possibly submitted external effect is `unknown`,
not retryable failed work. Unknown/interrupted operations retain conflicting
resource reservations until safe reconciliation. Cancel has its own mutation ID;
it does not rewrite the target operation's original intent/digest.

## External effects

Record each effect's provider correlation key, submission attempt, known state,
idempotency capability and observations. If submission may have reached an
external service, do not blindly resubmit after a crash. Query provider state
where supported; otherwise require an attributed reconciliation decision.
Final observations are monotonic and deduplicated; reused provider IDs with
changed content conflict. No exactly-once guarantee is claimed across providers.

The local revision/lease and current policy fence commits. Provider calls and
media graph changes are outside the database transaction. Compensation is an
explicit new action with observations; reverting a row is not compensation.
Worker leases use fencing generations; an expired worker cannot retire or
reactivate a connection after a newer operation commits.

## Retention

Operation receipts and digest tombstones remain at least as long as any referenced
external effect can be retried/reconciled. Default full operation records are
retained with the Conversation; a configured 30-day post-close receipt window is
the minimum advertised retry window. Expiry is explicit and cannot permit an old
ID to execute again silently: keep its digest tombstone and return gone after
full-result expiry. Operation IDs are not reused within an installation/tenant
epoch; explicit installation reinitialization establishes a new epoch.

## Current shared receipt boundary

Fresh schema 32 stores ordinary control receipts in the operation ledger, with
immutable typed origin, frozen recipient IDs and original actor membership epoch.
A message acknowledgement includes a local-mutation receipt and exact get status
reference. Its completed state means local message/journal acceptance; recipient
and provider delivery are distinct observations. A failed local mutation retains
a failed receipt without a completed result. Infrastructure failures roll back
the mutation and receipt together. Task internal wire receipts retain task/grant
origin and are unavailable through generic core status. Full provider/voice/SIP
effect integration, production canonical dispatch and Gate 6 acceptance remain
open; see docs/developer/control-receipts.md for the qualified scope.
