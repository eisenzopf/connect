# Private incoming contact routing

G6-045's approved incoming-routing work is implemented and qualified at schema
74. The current [clause audit and behavior evidence](../evidence/gate-6/incoming-contact-routing.json)
records 964 successful cases across the complete Store, server and client library
suites and the named provider, native/HTTP voice, schema and recovery integrations.
Three explicit Store subprocess helpers remain ignored by the top-level runner.
This closes the task's recorded local scope; Gate 6 implementation and acceptance
remain open, including the later designated physical-provider/remote profiles.

An authenticated provider resource is scoped by provider, channel kind, tenant,
configured account owner and exact selected account reference. A caller number
or provider ID is correlation data. Neither creates a principal, membership,
consent, a Session or a history grant. Reusing the same provider ID in another
configured account cannot merge its state. Conflicting content for an already
reserved resource is private reconciliation evidence, not another delivery.

A hold retains its original payload and candidate snapshots separately from
immutable event receipts and attributed decisions. Retry metadata cannot replace
original candidates. The private payload and any provider control capability do
not appear in administrative metadata responses. Current administration is also
restricted to the original account owner, tenant and recipient ceiling. It does
not grant access to a candidate Conversation's history. Inbox cursors are opaque,
installation-authenticated and bound to the current owner and scope; pages and
encoded source budgets remain bounded.

Holding creates no Conversation message or Session. Explicit rejection records
one attributed decision and a shared logical-operation receipt atomically, with
current authorization before replay. SQL retains the original resource, hold,
receipt and audit rows. A resource can acquire one delivery binding, tied to its
original candidate and a real corresponding message or incoming provider Session.
Rejected or conflicting resources cannot silently acquire a replacement binding.
SMS resolution requires the original verified Endpoint/account/route facts,
current delegate and account authority, original membership epochs/roles and the
same still-current consent grant. It writes an explicit original audience, so
new members do not receive a held `all` message. Metadata records the decision's
operator/credential, logical operation, selected candidate and local delivery
reference without granting access to the message body. Voice selection performs its own fresh authenticated original-resource read,
atomic attributed Session import and actual owned-worker pickup.

Both ordinary SMS admission and canonical voice discovery collect matching
configured candidates before creating a message or Session. Selection retains original candidates and commits actual delivery and
attribution together. Qualification covers the selected voice resource, competing/repeated decisions,
rollback/restart and public administrative refusal/visibility cases in the named
current suites. No legacy
command path or automatic Conversation merge is part of this routing contract.

Ordinary Telnyx input retains `payload.id` as the provider message resource,
separately from `data.id` callback identity and raw-event digest. Verified
STOP/START/HELP keep their existing control admission and provider-time ordering;
they never enter ordinary message admission. A unique eligible route can commit
private custody and its one real message in the same transaction. Multiple
eligible routes require explicit selection. Retained disabled, closed or
unmounted policies belonging to the same selected account still make a late
reply ambiguous; the remaining route is not chosen automatically. Conflicting
resource reuse stays quarantined. An originally ineligible resource is ignored
terminally and cannot become an assistant prompt when enrollment later changes.

Protected Rust composition supplies `SmsObservation` to `Store::receive_sms` or
`receive_sms_for_routes` only after actual signature verification. Unbound routes
cannot replace verified Endpoint/account policy. Trusted administrative SMS
resolution uses `resolve_incoming_sms_contact` with the original hold/candidate,
expected revision and stable operation ID. The named administrative interface below delegates to these Store methods; it
does not add a generic communication command alias. See the [callback HTTP reference](../../contracts/openapi-provider-observations.json).

Canonical voice discovery scopes a call resource to its configured provider
account and retains its immutable provider ID, assistant, phone-number resource,
caller and creation time. Later lifecycle state/control observations do not
change that identity. Each ingress checks all retained matching policies, current
verified Endpoints, configured credentials, exact membership epochs/roles and
original configuration time before any Session creation. Overlap is a private
hold rather than a forbidden configuration or a scanner-order choice. Retained
closed, withdrawn or unmounted policies prevent a late call from silently
choosing the one remaining Conversation. An existing hold keeps its original
candidates even if later scans see a narrower set. Schema 74 also prevents the
same configured provider account reference from acquiring another owner across
different caller numbers.

A uniquely routed voice call atomically records private custody, one common
Session invitation, its authenticated observed incoming resource, attributed
routing decision/audit and logical-operation receipt. The owned worker receives
the actual imported origin through its existing scan result. Discovery does not
accept a Participant, submit an outbound create, start local media, claim media
readiness or authenticate the remote caller. Reusing a resource ID with different
immutable call data is held privately; changing an already recorded event's data
conflicts and retains the original cursor. Rejected/ambiguous calls do not acquire
Sessions from later observations. Explicit voice selection uses a Store-issued ticket and a fresh, bounded
account-authenticated discovery of the original resource. The full original
provider ID, caller, assistant, phone resource and creation time must match;
absence or contradictory snapshots refuse selection. Current original
administrator/owner, Endpoint/account bindings and membership epochs/roles are
checked again after IO. The decision, common incoming invitation, observed
resource, audit and receipt commit together. The host queues the resulting
opaque origin on its owned lifecycle; a database row alone is not worker pickup.
Resolution jobs are bounded and drained with the provider host. Caller reply
cancellation cannot detach an admitted job or authorize another creation.

## Administrative HTTP and public client

The main administrative listener accepts `POST /v2/incoming-contacts`; the
callback-only listener does not expose it. See the closed
[HTTP/OpenAPI reference](../../contracts/openapi-incoming-contacts.json).
Authentication and current `admin` authority precede body decoding. Requests with
an `Origin`, `x-call-id` or `x-chat-id` header are refused. The body limit is
64 KiB, HTTP admission has a five-second budget, responses use `connect/2` and
`Cache-Control: no-store`. This interface requires the original account owner,
tenant and current recipient ceiling, even for historical decision replay. It
confers no Conversation history access.

Supported JSON commands are:

```json
{"command":"list","after":null,"limit":32}
{"command":"get","contact_id":"ich_original"}
{"command":"resolve","operation_id":"choose-incoming-01","contact_id":"ich_original","candidate_id":"icc_original","expected_revision":0}
{"command":"reject","operation_id":"reject-incoming-01","contact_id":"ich_original","expected_revision":0,"reason":"Not for these Conversations"}
```

Use `ApplicationClient::incoming_contact_command` or its cancellation variant
with the explicitly selected main-listener configuration. The client bounds
responses, checks the profile and closed DTOs, and correlates decision operation,
contact, selected candidate and kind-specific local message/Session reference.
The client never retries automatically or falls back to a communication command.
An unsuccessful or timed-out reply can follow accepted work. Keep the complete
original decision command and operation ID and replay it, including the original
`expected_revision: 0`; changing the revision or selected candidate changes the
intent. Held metadata has revision 0 and no decision; resolved/rejected metadata
has revision 1 and immutable decision attribution. `operation-pending` means the
owned decision is still running and must not be replaced by a new ID.

SMS selection requires the same current original consent and sender binding;
STOP or authority withdrawal also refuses a completed decision replay without
redelivering the message. A voice replay checks current original policy before
returning its recorded decision and does not require another provider read. New
voice selection reads only the original resource window and never dials, accepts
a Participant, creates a media Connection or claims readiness. If an admitted
voice result cannot be queued during teardown/capacity refusal, the original
resource stays durable and existing owned cleanup/recovery handles it; no second
provider creation is authorized.
