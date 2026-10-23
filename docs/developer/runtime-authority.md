# Runtime authority changes (G6-037, in progress)

The [owned runtime continuation](owned-runtime.md) records current schema-31
startup ownership. Earlier evidence reports retain the schema/source state they
qualified; they are historical checkpoints.

Schema remains 31. The [partial writer inventory](../../contracts/gate6-authority-writers.json)
and [local evidence report](../evidence/gate-6/runtime-authority.json) extend the
[ordinary/task writer checkpoint](authority-writers.md). Production policy,
complete writer coverage and the canonical listener remain open.

Provider job claims, completion, normalized receipts and independent inbound
messages now derive current write scopes from persisted job/route records before
SQLite admission. A copied ProviderJob cannot select a different stored route
when finishing. Provider, voice and SIP recovery retain unknown outcomes and do
not repeat a possibly sent creation. Actual network I/O still occurs outside
SQLite; this slice fences database transitions, not the whole remote dispatch.

Voice/SIP claims, last-mile persisted dispatch admission, preparation, results,
Connection end observations, scans and inbound voice import acquire their exact
Conversation/member write sets. The current target set is checked again in the
original transaction before any state, receipt or context write. A different
job can become eligible while a worker waits, including because time advanced.
An uncovered candidate produces authority-unavailable with no mutation; the next
claim derives fresh authority. A deterministic test crosses the real poll time
boundary between two Conversations and proves the refusal and subsequent poll.

Raw voice results cannot reopen a terminal call or change an already bound
provider effect ID. Conflicting facts require attributed reconciliation rather
than inventing a second call. Normalized observations keep their existing
monotonic projection/reconciliation rules. SIP cancellation observations still
cannot replace starting/ending/uncertain cleanup with an unproven terminal fact.

Operator route/ingress changes check current administrator and tenant before
waiting and inside the original transaction. Both current and proposed route
Conversations are covered; bindings remain immutable except enable/disable.
Wildcard recipient scope does not permit a voice route into another tenant.
Protected Vapi task setup/possible-send admission also retains its original
current Task, Session, caller and immutable intent checks under a write fence.

SMS snapshot refresh covers old and proposed grant Conversations; removal and
cached authority expiry wait for held delivery readers. STOP is keyed globally
by phone, so its footprint includes other senders and Conversations. A test
holds another sender's independently owned Conversation and proves that STOP
waits, then suppresses that sender too. Refresh never clears opt-out evidence.
Static protected queries and merged scope derivation refuse more than 1024
unique scopes; no SQLite mutex or global grant mutex spans the file wait or a
network await. Duplicate Conversation IDs are collapsed before member lookup.
Bulk profiles exceeding these bounds remain unqualified; this is not a claim
that recovery of every configured state size is complete.

A real signed loopback Telnyx STOP returns 503 while an owned delivery guard is
held, with no event receipt or policy change. The exact same event data/ID returns
204 after release; controls never become assistant prompts. The callback verifier
still checks the original bytes and timestamp before parsing. The
[provider callback HTTP reference](../../contracts/openapi-provider-observations.json)
documents these responses, ignored events, required transport signatures and
current controls. This fixture uses generated local state and fake endpoints;
it does not perform a live provider operation or prove provider redelivery.

The [operation worker checkpoint](operation-authority.md) adds selected operation
metadata writers and bounded recover/fence/purge batches. Remaining work includes
owned runtime startup, video reconciliation, other retention/natural expiry, complete
reader/held dispatcher scope derivation, retirement watchers, exact signed-key
Device admission and actual response/network writer permits. Existing legacy
native regressions do not qualify canonical listener or media cleanup. No Linux,
remote/browser/live-provider, full canonical QUIC/HTTP/SDK parity, publication or
Gate 6 acceptance follows from this local writer integration.
