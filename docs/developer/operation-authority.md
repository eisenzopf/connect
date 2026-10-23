# Operation worker authority (G6-037, in progress)

The [owned runtime continuation](owned-runtime.md) records current schema-31
startup ownership. Earlier evidence reports retain the schema/source state they
qualified; they are historical checkpoints.

Schema remains 31. This extends the [runtime writer checkpoint](runtime-authority.md)
and the [partial writer inventory](../../contracts/gate6-authority-writers.json).
The [local qualification report](../evidence/gate-6/operation-authority.json)
records executable results and unchanged source digests. Production listener,
reader/delivery policy and Gate 6 acceptance remain open.

Operation admission and replay, Get's lazy recovery, cancellation, reconciliation,
worker claims/renewal/advancement, effect preparation/submission intent/observation,
evidence recording, cleanup claims and abandoned-owner settlement acquire owned
write fences before taking the SQLite mutex. The footprint is the saved owner's
Identity plus its optional Conversation. These metadata changes update the
owner's receipt and operation state; they do not change other member Identities.
Protected graph/role commit callbacks retain their broader Conversation/current
member write set. Generic operation receipts include provisioning and membership;
footprint derivation never assumes every saved row contains a TransitionRequest.

Derivation checks the current credential, scope, owner/tenant and original source
policy. Those checks run again in the original transaction, where the saved row's
actual footprint must still be covered before any mutation. New transition intent
uses current admission/source checks; retained replay uses its original receipt
policy. A cleanup operator can differ from the abandoned owner, so settlement
also fences the operator's Identity for its new receipt. Its explicit cleanup,
Session-read, tenant, recipient ceilings and attributed settlement checks remain.

`execute_operation_action_with_authority` permits protected composition with a
sufficient proof from the same installation. A read, incomplete or foreign proof
fails without fallback, upgrade or reentry. Tests retain the proof while a real
revocation writer queues, complete cancellation, then release it to let revocation
finish. Current revoked credentials cannot use a matching receipt afterward.
Other transition/worker entrypoints still need held-proof composition when the
production dispatcher and coordinator are installed.

Recovery, worker fencing and result purging select a deterministic eligible prefix
of at most 1,000 rows whose union contains at most 64 write scopes (at most 128
held admission/resource files). The prefix uses owner/Conversation metadata,
without decoding intents or loading effects to decide the footprint. It is derived
again after the bounded file wait; the immediate transaction rechecks eligibility
and scope for every selected ID before mutating any row. It never substitutes a
new global selection after admission. Startup drains while a batch returns more
than zero, because scope-limited batches may be smaller than the requested limit.

The large-state fixture constructs 1,030 historical workers with valid source
admission, frozen membership and distinct Conversations. It checks scope-limited
prefixes, `open_for_recovery` draining worker fences and normal open draining
business recovery. Possible submissions remain unknown with their original
attempt and no resubmission. Administration recovery does not adopt live foreign
workers; they cannot occupy its prefix and starve a recoverable expired lease.
Another test makes a second row eligible after planning and checks that only the
original selected ID is processed under that batch's proof.

Purge candidates exclude unresolved effects before limiting the batch, so an
old retained row cannot starve later purgeable results. Closed/terminal retention
windows, permanent ID/digest tombstones and confirmed compensation requirements
are preserved. Tests cover waiting without SQL changes, multiple batches and
retention of an oldest unresolved result. This qualifies these selected worker
batches, not every provider/SMS/retention capacity profile.

Store startup still needs explicit owned runtime authority before connection epoch
fencing and video reconciliation. Database fences around submission intent do not
retain authority through actual network/body admission or uncancellable work.
Complete caller/resource reader scopes, natural expiry, task/transition held-proof
composition, exact signed-key Device validation, retirement watchers and actual
response/network writer permits remain open. No remote, Linux, browser,
live-provider, media-cleanup, publication or Gate 6 acceptance is inferred.
