# Participant endpoint routing

G6-032 adds the Rust SDK's `participant_route` read operation to the existing
shared service. The same operation works over native UCTP and the current
loopback HTTP facade, `POST /v1/operations`. Its implemented subset is described
by [OpenAPI](../../contracts/openapi-routing.json). Canonical command dispatch and
general Session allocation remain G6-037/G6-038 work; this lookup is not admission.

Obtain a conversation appearance from
`client.capabilities(address, Some(conversation_id)).await?.participant_id`.
The returned Participant ID is distinct from the durable Identity ID. Without a
Conversation, the capability response has no Participant ID. Construct a
`connect_client::routing::ParticipantRouteRequest` with that appearance, purpose,
direction, complete requested media, allowed transports, an optional device
preference, failover policy and reachability policy. The request has no provider
route ID, destination address, credential reference or approval assertion.

The resolver checks the current caller credential, tenant, Conversation and target
membership epoch before selecting its bounded endpoint inventory. It requires
`resolve`, `availability_get` and the communication permission: outgoing messages
use `send`, incoming messages use `inbox`, outgoing Sessions use `session_invite`,
and incoming Sessions use `session_accept`. File use additionally requires
`artifact_begin` or `artifact_get` respectively. Outgoing telephone/SIP selections
also require `call_start`/`sip_dial`. Incoming lookup may select only the caller's
own participant. Roles and client-selected identifiers do not confer permissions.
Cross-tenant identity, capability and availability discovery is forbidden even
with a wildcard recipient ceiling.

Selection checks live identity/device ownership, endpoint verification, complete
endpoint and runtime capability support, operator configuration, current route
delegate authority and any channel-specific consent. The response distinguishes
`unverified`, `unsupported_capability`, `unconfigured`, `permission_required` and
`consent_required` from `offline`, `busy` and `unknown`. Mixed prerequisites are
reported as `prerequisites_missing`, with each candidate's actual reason. An
inaccessible participant or device returns forbidden without an inventory.

## Preference and unknown reachability

Two usable endpoints are ambiguous. No lexical ordering chooses one silently.
An explicit preferred Device restricts selection. If it becomes offline or busy,
`failover: none` preserves that result. `other_available_device` permits another
fresh reachable Device of the same Participant, within the expressly requested
transports and full media set. An incompatible video request is never downgraded
to audio; a native route never silently becomes a SIP/telephone/SMS route.

`reachability` defaults to `require_fresh`. An absent or expired observation is
`unknown`, not offline or reachable. `allow_unknown` explicitly permits an attempt
to a verified and otherwise permitted endpoint whose reachability is unknown;
the result is `attempt_allowed`, and its selected candidate remains `unknown`.
This supports verified phone endpoints whose reachability is learned by calling.
It never bypasses known offline/busy state, credentials, capability, configuration
or consent. Unknown alternate Devices do not become fresh reachable failover
targets. Multiple eligible unknown endpoints still require disambiguation.

Public results contain only endpoint/device IDs, endpoint revision, participant
epoch, Conversation revision and bounded reachability metadata. They contain no
phone numbers, URI addresses, provider route IDs, secret references or secret values.
The legacy capability route list remains configuration metadata; migrating the
remaining route-ID-oriented facades is part of the later canonical API/SDK work.

## Trusted runtime observations

`EndpointRuntime` contains an immutable supported transport/media profile and a
bounded, process-local observation cache. `Service::observe_endpoint` is trusted
adapter composition, not a public command or a deserializable callback DTO.
Adapters publish only actual endpoint observations, with installation ID,
endpoint revision, sequence, observed/expiry times and attributed source category.
A provider being configured or healthy is not evidence that a particular endpoint
is reachable. No automatic provider probe is performed by discovery.

Capture `Service::endpoint_observation_epoch()` when the observer starts. Never
replace that captured value when a delayed callback arrives. Every runtime has a
fresh epoch, including a server stop/start within the same process. The store
rejects the wrong installation, old generation, changed endpoint revision,
unverified/revoked ownership, future timestamps and leases longer than 60 seconds.
Reordered sequences cannot regress state. Cloned runtime handles share the same
generation/cache. Restart and backup restore do not restore live observations.

The cache holds at most 4096 endpoint observations. Expiration does not erase the
sequence fence or silently evict another endpoint to admit a replay. Capacity
failure is explicit. A participant inventory is bounded to 64 endpoints; exceeding
it fails instead of using a truncated set that might hide ambiguity.

Runtime credential references identify constructed bindings and are scoped to the
transport. Their names contain no credential values. The current server registers
`env_TELNYX_API_KEY` for its SMS adapter, `env_VAPI_PRIVATE_KEY` for available Vapi
chat/voice/telephone adapters, and `config_sip_outbound` for configured SIP
origination. An injected Vapi test adapter occupies the same voice binding slot.
Other/mismatched references are unconfigured. Trusted hosts can construct their
own explicit reference map. A configured SMS endpoint still needs a fresh reviewed
enrollment and absence of STOP. A telephone endpoint's owner is the person being
dialed; its configured voice route can identify a separate permitted assistant.
The resolver does not relabel the assistant as the phone owner.

## Admission and qualification boundary

Selection is a snapshot, not a reusable permission or physical media result.
Trusted code calls `Store::authorize_selected_endpoint` after asynchronous
preparation and immediately before use. It reruns current policy and requires the
same selected snapshot before exposing the private address/reference to the
trusted runtime. Revocation, expired hints, changed preferences/policy or a new
generation can require fresh resolution. Ordinary clients cannot materialize the
private binding through the operation API.

`scripts/check-gate6-routing.py` records the isolated store, native SDK/HTTP,
OpenAPI sample and regression evidence. Native control requests are real TLS/QUIC;
endpoint observations are explicitly local fixtures. This does not qualify a
physical endpoint dial, real provider reachability, multiparty media admission,
browser/WebTransport sessions, remote hosting or full Gate 6 acceptance. Later
Session/transport adapters publish their actual attributed observations and consume
the fresh selection at their own admission boundary.
