# Owned public assistant hosting

Protected Rust composition can install `PublicVapiCoordinator` on the canonical
Service with explicit `PublicVapiTarget` profiles. Each target names its own
signed native peer, verified Vapi voice Endpoint, provider/model/application
profile, constructed routing runtime and reachability policy. Installation adds
no wire permission, provider installation command or model execution tool.
Canonical TOML can install the same coordinator before public intake, using the explicit configuration below.

The shared Service registers hosts with
`install_session_initializer(name, host)`. Distinct local names permit public AI
and telephone coordinators to coexist. The registry is bounded by sixteen hosts
and the configured Connection limit. Notifications carry only committed Session
metadata; each host still checks its own exact configured target and current
source authority. A blocked or failed notification does not skip another host.
Cancellation closes registration under the registry lock, refuses later
notifications, and reaches every host before parallel draining begins. A queued
snapshot rechecks closure before entering the owned work set. One cleanup
failure does not prevent joining another host. Host registrations and their owned
job handles remain available for a later drain after uncertainty.

A completed `session.accept` from the configured AI's own current credential
queues hosting metadata. The same hook serves signed QUIC and HTTP commands.
Invitation alone never auto-accepts. The host selects the exact current active
AI Participant and membership epoch; it does not choose the first invitee,
borrow the Session owner's credentials or treat a provider reference as identity.
A Session receipt confirms the participant decision, not provider/audio readiness.

The host opens a fresh signed native peer, prepares its own mono PCMU Connection,
and selects audio only from its authorized current public audience. Other
Participants retain their independent subscriptions. The host rechecks its
configured provider Endpoint through ordinary Participant routing, including
current verified ownership, route/configuration, capabilities, secret references,
availability and reachability. `AllowUnknown` is an explicit host policy permitting
an attempt; it never reports the provider reachable. Missing profiles and strict
unknown reachability refuse before provider I/O.

Initial routing policy stays with the captured provider context. The Store
rechecks it when preparing the durable context and before the first possible
creation under the actual native source's current authority interval. An incoming
availability hint does not erase an already owned remote call. Existing provider
memory and every export still require the original current credential, recipient
ceiling, membership epochs, actual native activation and exact Endpoint binding.
Public hosting cannot use a private consultation's purpose or export its memory.

One bounded registry owns jobs by Conversation, Session, AI Identity and
membership epoch. Native offer, subscription and provider context operation IDs
are deterministic for that job. Exact acceptance replay cannot create another
provider instance. The existing durable provider attempt and exact context ACK
still govern creation, readiness, uncertainty and cleanup. A successor must not
adopt an old physical incarnation or retry uncertain creation under a new ID.

A job reports `Preparing`, `ContextReady` or `Stopped` with a bounded problem
code. Known initialization refusal is separate from unconfirmed physical cleanup.
The server cancels and drains public jobs before closing peers or retiring Store
authority. Drain has one ten-second budget and retains original task handles on
timeout for another join; it does not detach them. Unconfirmed cleanup remains an
error. Native audio is closed and joined when context preparation refuses too.

The job's operation result and its cleanup proof are separate. Authority loss
still reports a stopped job with its original problem. Drain succeeds only when
a known lack of provider submission (or an authenticated, recorded terminal
provider outcome) and the owned native endpoint drain are confirmed. A failed
operation alone neither proves nor disproves resource cleanup. Missing provider
terminal evidence remains an error on repeated drains; joining a completed task
cannot erase that uncertainty. Peer/context/profile refusal also closes and joins
the exact supplied native peer after releasing its context reader.


This is an optional programmatic hosting component. It does not complete the
full G6-044 provider/endpoint mapping, automatic acceptance policy, canonical
telephone workflow migration, whole fixture matrix or
named live-provider qualification. The retired public voice fixture remains an
explicit unresolved test set. Live Vapi voice initialization, uncertain creation,
duplicate admission, withdrawal/restart and teardown must rerun under the concrete
G6-093 configuration and designated endpoints. Defaults remain isolated fixtures.

[The local verification report](../evidence/gate-6/provider-public-supervision.json)
records 759 fresh passed tests: the full Store suite, server units, five actual
hosting fixtures and existing general Session/private AI regressions. Three Store
subprocess helpers remain explicitly ignored as standalone harness entries. The
hosting fixtures cover signed and HTTP acceptance, exact context acknowledgment,
actual native audio into the provider, duplicate acceptance, joined shutdown and
zero provider I/O for missing context, missing runtime and strict unknown
reachability. They do not qualify production startup, discovery parity, public
hosting restart/uncertainty or the full provider matrix.

## Canonical startup configuration

`CanonicalServerConfig.public_vapi` selects protected native peer and secret
reference files; it contains no provider secret values or participant-supplied
model settings. Relative paths below resolve from the server TOML directory.
Paths inside the native peer file resolve from that file's own directory.

```toml
[public_vapi]
secret_references = "private/provider-secrets.toml"

[[public_vapi.targets]]
native_peer = "private/assistant.native.toml"
provider_endpoint = "the_verified_vapi_voice_endpoint_id"
secret_ref = "PUBLIC_VAPI"
api_base = "https://provider-api.example.invalid/"
websocket_origin = "wss://provider-media.example.invalid/"
model = "operator-selected-model"
application_prompt = "Use only the supplied authorized public history."
context_timing = "after-creation"
reachability = "require_fresh"
```

The `.invalid` origins are placeholders. Select the exact authorized API and
WebSocket origins from the bounded provider configuration. Plain HTTP/WS requires
an explicitly enabled numeric loopback fixture. `context_timing` is
`after-creation` or `initialization`. `allow_unknown` is an explicit attempt policy
for unavailable reachability observations; it never marks a provider reachable.

The references file uses the existing strict `SecretReferences` format:

```toml
version = 1
[references.PUBLIC_VAPI]
storage = "environment"
variable = "OPERATOR_SELECTED_VAPI_KEY"
```

An explicit `keychain` reference is also supported on macOS. The references/native
credential/private signing-key files must be regular protected files with no group/other access (normally 0600
on Unix). Server target entries accept paths/references, never inline provider
keys or peer credentials; symlinks and unknown fields are refused. Missing,
locked or absent selected references cannot use another entry or an ambient
provider key. Unselected entries are validated as descriptors, never resolved.
Secret values are checked at startup and resolved again through the selected
reference for actual jobs; changing target or reference configuration requires
restart. A native peer's local socket address is rebound to this exact listener,
while its pinned server key, algorithm, validity window and certificate must
match. Its private key must match its current enrolled credential/Device key.

Startup checks the current AI Identity, credential scopes, native Device/key,
exactly one verified native QUIC Endpoint supporting audio and the exact verified Vapi voice Endpoint/secret binding
before installation. The resulting discovery runtime is shared by the public
`participant_route` and capability reads and provider admission. It advertises
constructed native QUIC media profiles and only configured Vapi Endpoint IDs.
Another Endpoint sharing the same Vapi transport or secret reference remains
unconfigured. Multiple verified QUIC Endpoints on the configured Device are
ambiguous even if the extra Endpoint supports only data; configuration refuses
that ambiguity before provider IO. Current credential/key/Device/Endpoint checks
repeat on discovery and admission; revocation cannot retain a configured Vapi
route. These are configuration observations, not successful provider probes.
Messaging, PSTN control, SIP initial-call, video-provider and remote profiles are
not implicitly enabled by a public native voice host.

Optional `observations` declares transcript/speech capability and up to sixteen
unique client proposal action names; it requires the configured native credential's
current `session.observation.publish` permission. These are untrusted typed
observations, not server execution tools. Opaque task question bindings cannot be
manufactured by TOML fields.

The startup/discovery checkpoint and its exact local fixture scope are recorded
in [`provider-public-startup.json`](../evidence/gate-6/provider-public-startup.json).
It covers TOML startup and current configuration refusals; broader provider
operation integration and Gate 6 acceptance remain open.
