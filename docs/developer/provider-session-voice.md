# Session-owned provider telephone control

G6-044's `provider_sessions` Store boundary attaches normalized telephone
provider effects to an ordinary, completed canonical `session.invite`. It does
not create an application command, a Task wrapper, or a local-media Connection.
The connector manifest explicitly separates telephone start/status/end control
from its independently negotiated live-media adapter.

The canonical server has an optional `provider_voice.routes` configuration. Its
owned Session initializer selects an explicit telephone target only when the
initiating Participant is the configured, verified AI Endpoint's own identity.
The worker uses the configured account's actual key reference and a bounded
normalized adapter; ordinary callers continue using Session commands. Existing
Session and operation commands are listed in the
[canonical HTTP reference](canonical-http.md). The broader G6-044 integration remains open. `voice_flow` now uses canonical
Session commands for its five original functional requirements; the additional
inbound and capability cases have their own local behavior evidence.

## Original invitation and Endpoint

`prepare_provider_session_voice` requires the caller's exact successful invite,
an explicitly invited Participant, current original membership epochs and
Session audience, a verified telephone Endpoint and Device, its own current
Endpoint credential, and the selected route account/reference. The Endpoint
credential must authorize its own connection offer and Session acceptance; the
route account authorizes connection offers for that audience. The reference
must match the Endpoint's protected secret reference. Resolved secrets remain
with the adapter.

Schema 67 reserves one provider-call identity for invitation plus target. Replay
retains the original policy and deadline; a changed policy conflicts. SQL guards
bind the original control receipt, target and Endpoint, retain their identities,
and prevent the same invitation target from acquiring both initial SIP and
provider-control hosts.

Schema 68 additionally binds the configured assistant to its current verified
Vapi voice Endpoint, Device, owning AI identity and credential, assistant address,
route owner and secret reference. A provider assistant ID alone grants no
speaker identity. Both endpoint credentials and the route account must permit
the original audience. Observer roles are refused because this control profile
cannot project live audio roles; changing a participant's role while its
provider call remains open is explicitly unsupported.

## Owned listener integration

Each protected route includes `endpoint_id`, `endpoint_credential_id`,
`assistant_endpoint_id`, `assistant_credential_id`, `route_credential_id`,
`route_reference`, `provider` (`vapi-telephone`), `assistant_id`, `phone_number_id`
and a `timeout_seconds` of 1–30. Startup resolves the actual configured secret
reference and constructs its normalized Vapi driver. Ambiguous routes for one
AI/telephone identity pair, and automatic initial SIP/provider routes for the
same telephone identity, are refused before admission.

Only the worker owns provider IO. Each create/status/end attempt is bounded by
its dispatch deadline (at most two seconds); a timed-out create remains unknown
and is never redialed. The worker checks original permission while starting;
after observing an active call it monitors the independent configured AI,
telephone and route credentials, original epochs and current Endpoints. Session
End/Cancel and host cancellation stop new work and clean up the exact original
resource. Draining joins admitted jobs before retiring its exclusive worker
role. A replacement host must acquire that role and recover pending original
effects; unresolved work remains visible as a drain failure.

This control profile has no native/media bridge or model-context export.
`provider_voice.ingress` explicitly enables incoming observation for selected
Conversation/Endpoint bindings. A configured ingress remains dormant if its
adapter does not declare `VoiceDiscover`; existing outbound/status/end control
still works. A profile with discovery/status/end and no `VoiceStart` can import
incoming calls and clean their exact original resources; outbound creation is a
typed pre-dispatch refusal. Positive recovery of an unknown outbound create,
context grants and the remaining provider matrix stay open.

## Authenticated incoming discovery

An incoming binding has an `id`, `conversation_id` and nested `policy` with the
same protected assistant/telephone/account fields. Its intended AI and telephone
Participants must already belong to that explicit Conversation. The verified
telephone address is an eligibility filter, not authentication of the reported
caller. No incoming receipt creates an Identity, adds a Member, accepts a
Participant, exports history or grants media. Public facts label their direction
and always report `remote_identity_verified: false` and `local_media: false`.
The `participant_id` is the intended invited Endpoint recipient, not a claim
that this remote caller authenticated as that Participant.

```toml
[provider_voice]
routes = [] # Incoming-only; does not enable automatic outbound dialing.

[[provider_voice.ingress]]
id = "support-incoming"
conversation_id = "configured-conversation"
[provider_voice.ingress.policy]
endpoint_id = "verified-telephone-endpoint"
endpoint_credential_id = "telephone-credential-id"
assistant_endpoint_id = "verified-ai-endpoint"
assistant_credential_id = "ai-credential-id"
route_credential_id = "account-credential-id"
route_reference = "configured-secret-reference"
provider = "vapi-telephone"
assistant_id = "configured-provider-assistant-id"
phone_number_id = "configured-provider-phone-id"
timeout_seconds = 30
```

Schema 69 retains the original incoming activation time and monotone discovery
cursor. A worker acquisition has its own ID; only that exclusive worker can
activate/query/import its configured bindings. Configuration replay retains
`since` and the cursor. Endpoint/Device, credential permissions and original
Participant epochs are checked before inspecting duplicates. Two current
bindings for the same account/assistant/phone/caller number refuse ambiguity;
G6-045 still owns private ambiguous-contact holding/resolution. Calls outside the
explicit configured address policy are not routed or turned into principals.

Each read chooses a fixed upper bound no more than one minute beyond its last
cursor and no later than local wall time. The Vapi adapter re-reads its bounded
overlap and rejects truncated/unsplittable discovery. Discovery dispatch is
bounded by the configured timeout and credential expiry. Failed/malformed
batches cannot advance the durable cursor.

Import uses the ordinary current Session invitation engine. Stable operation and
Session IDs bind the original ingress/provider resource. The normal invitation
receipt, incoming provider observation effect, call projection, scoped journal
and cursor commit in one SQLite transaction. Capacity/storage/current-policy
failure rolls all of them back. Import records a confirmed `status-query`
observation effect; it never submits provider creation. A matching duplicate
neither creates another Session nor changes the existing lifecycle/private
capability; its owned status poll handles updates. An existing resource in a
different direction/binding cannot be reattributed. Already-terminal incoming
history does not reserve an active call slot, and records terminal cleanup
without sending End.

Scanner cancellation stops new provider reads and joins admitted work. A resource
committed during cancellation is queued under the original identity or cleaned
inline before the worker role retires. Restart resumes original known resources
and the same discovery cursor without redial. This is provider telephone control;
it does not qualify caller identity, actual live provider/PSTN operation,
provider context export or a native audio bridge.

## Dispatch, uncertainty and cleanup

The worker role is exclusive to the actual authority manager and runtime.
Transport and captured-result custody retain it after worker retirement. A new
worker cannot run recovery while the previous worker still owns those handles.
Recovery is serialized with dispatch, runs before the new worker dispatches,
and preserves interrupted creation/end as unknown. It never repeats either
mutation. Protected enumeration/resumption restores the original binding and
does not renew its start deadline.

A creation marker and submitting effect commit on the original invitation
operation before `begin_provider_session_voice` returns a dispatch. Its current
caller, own Endpoint and route authority cover a bounded interval of at most two
seconds. The host must bound the actual provider request by that deadline and
check the dispatch immediately before IO. Capturing its result releases the
reader before any result writer waits. Historical response custody does not
renew the command grant.

Status and cleanup address only the original attributed resource. Cleanup owns
a separate stable compensation effect. An uncertain creation without a provider
ID cannot be blindly ended or redialed. An interrupted cleanup is not repeated;
a subsequently reconciled terminal observation can settle its original cleanup
effect. Confirmed creation and confirmed cleanup are distinct facts. Revocation
can prevent new work without erasing an admitted response or preventing owned
resource cleanup.

An adapter may separately declare `VoiceReconcile`, requiring status and End
capabilities without requiring current creation permission. Only an original
outbound effect already recorded as unknown, with no attributed resource or
binding quarantine, can obtain this read-only dispatch. Its query retains the
original assistant, phone and destination, with an inclusive window from the
original grant expiry minus its configured timeout and sixty seconds through
that same expiry plus sixty seconds. Restart never moves that window. The
additional minute on each side bounds provider clock skew and indexing delay;
results outside the window remain unresolved.

Vapi creation sends a forty-character opaque correlation in its documented
call `name`. Reconciliation lists the authenticated account with assistant,
phone and creation-time filters, splits saturated windows without dropping
timestamp ties, and requires exactly one matching correlation, outbound call
type, assistant, phone, destination and original time window. It then gets that
resource again and rechecks the full binding before returning explicit
reconciliation evidence. Request count and decoded history have fixed budgets.
See the provider's [create](https://docs.vapi.ai/api-reference/calls/create) and
[list](https://docs.vapi.ai/api-reference/calls/list) references. This correlation
identifies an effect within the configured trusted account; it authenticates no
remote principal and exports no conversation context.

An empty, truncated, ambiguous, changed or failed lookup preserves the original
unknown effect and emits no fabricated provider observation. It never proves
absence, releases capacity or authorizes another create. A positively correlated
resource confirms the original effect; terminal evidence also settles its
separate cleanup compensation without sending End. Otherwise cleanup sends End
only to the newly attributed original resource. The original Session receipt
remains unchanged. Missing `VoiceReconcile` retains the explicit unresolved
outcome. An existing binding quarantine still requires its own typed resolution;
ordinary reconciliation cannot overwrite or clear it.

An authenticated provider ID is still only attribution. If that ID already
belongs to another call in the configured provider/account/reference namespace,
the new receipt is retained privately in quarantine. Its provider ID and control
URL do not become dispatchable on the second call. Even an unattributable
terminal receipt cannot settle that call or release its quota. A protected
`ResolveQuarantine` dispatch takes a typed ticket bound to that original
conflict's digest. It uses the same bounded authenticated correlation lookup,
but requires separate admission from ordinary reconciliation. A missing,
unverified or still-claimed candidate leaves the conflict unchanged. An operator
assertion or ordinary control receipt is not a resolution proof.

Positive evidence for a distinct, unclaimed resource records an immutable
`provider_voice_quarantine_resolutions` row and binds the original effect,
projection and journal in the same transaction. The original conflict remains
immutable private history. SQL also refuses attaching a resource to a
quarantined call without that matching proof row. Commit failure rolls everything
back, allowing the same read-only evidence to be recorded again. Competing
captures cannot replace the first resolved resource, and replay cannot reopen a
later terminal state. Terminal resolution confirms separate cleanup without an
End request; nonterminal resolution allows cleanup only of its original resource,
including after caller revocation. The canonical worker obtains this separate
ticket when ordinary correlation encounters an unresolved binding conflict.

The public binding problem clears only when a matching committed resolution
exists. Both the current resource and retained quarantined provider references
stay private, including when reflected in an ended reason. A resolution changes
no principal, membership epoch, Session acceptance, media readiness or original
Session receipt. Snapshot inspection includes the retained resolution history.

## Observations and visibility

Normalized observations check mutation/resource correlation and verification,
retain duplicate identities, reject conflicting observations and require explicit
reconciliation after uncertainty. Older observations cannot replace the current
status or private control capability. Repeated metadata settlement does not
repeat physical control or emit another provider-call revision. The original
Session control receipt, including its result, timestamps and revision, remains
immutable. Provider-call revisions and audience-scoped journal entries advance
independently; operation effects retain the original invitation correlation.

Typed `provider_call` facts are carried by existing conversation snapshots and
`provider_voice` journal events. They identify the original operation, Session,
target Participant/epoch and verified Endpoint, with an independently revised
state (`starting`, `active`, `ended`, `failed`, `unknown`), inbound/outbound
direction, explicit lack of remote principal verification, optional observation/
closure times and refusal code. `local_media` is always false. Native and HTTP
use the same Store projection; the SDK validators and OpenAPI describe this
closed shape. Private provider IDs/control capabilities are absent.

The authorized view requires current Session-read authority, original audience,
and the same Participant/identity/membership epoch. A legitimate current role
change does not erase historical visibility. It also cannot reuse the original
start authorization. Provider IDs, private control URLs and quarantined receipts
are excluded from that view; private identifiers/URIs reflected inside a provider
ended reason are omitted from public snapshots/journal while their raw bounded
evidence remains private and redacted from opaque custody types.

No status receipt generates `session.accept`, a media grant, a native appearance,
or `ConnectionReady`. Ordinary own-identity acceptance, actual media admission,
context delivery and negotiated bridge capabilities remain separate work. The
Store fixtures exercise synthetic normalized receipts at that boundary. The
canonical `voice_flow` fixtures exercise actual QUIC/HTTP listeners and public
Rust SDK commands with a synthetic normalized adapter. Neither qualifies real
provider IO, live media, inbound caller identity or remote/provider operation.
