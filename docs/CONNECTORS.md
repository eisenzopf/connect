# Contributing a connector

Start with the public `connect-connector-api` crate and the
[versioned contract](../contracts/connectors/profile-v2.md). A connector is trusted
code selected by the server owner at composition time. There is no runtime plugin
loader, remote download or marketplace. A fixture contribution is executable
teaching material, not another supported provider.

## Build and check the example

From the repository root, prepare the pinned dependencies and run the standalone
package with an isolated fixture. No provider account is needed:

```sh
bash scripts/prepare-dependencies.sh
cargo test --locked --manifest-path examples/connector-template/Cargo.toml --target-dir .local/gate5/connector-template-target
python3 scripts/check-connector-contract.py --output .local/gate5/connector-report.json
bash scripts/format.sh --check
```

The template is its own Cargo workspace. Its library imports the public connector
API, ordinary serialization/async libraries and a fixture signature implementation.
It imports no Connect store, service, server, conversation or routing internals.
`src/lib.rs` implements messages/context and provider fixture persistence;
`media/voice.rs` implements bounded voice lifecycle and PCM framing.
Its state file represents the remote provider's journal, never Connect's history.
Keep fixture state, logs and build targets under ignored `.local/`.

For a clean public-contract exercise, export a kit into a new directory:

```sh
python3 scripts/check-connector-contract.py --export-kit .local/gate5/public-connector-kit
```

This exports API sources/tests, this guide, the profile and source digests. It
contains no server/store/service code or author fixture implementation. Create a
new Cargo package depending on the exported `crates/connect-connector-api` path
and implement your own adapter. A published API package can replace that path.
The export flattens workspace metadata only; it retains exact public Rust source.

## Implement the boundary

1. Implement `Connector` and declare `Descriptor` plus `manifest()`. The manifest
   must agree with dispatch's address/context/SMS semantics. Populate the typed `operations` set, including status/reconciliation and
   media exchange where implemented. Declare only operations
   and codecs you actually implement. Manifest v2 distinguishes unsupported media,
   a PCM lifecycle fixture and an actual live binding; see the media contract below. `configuration()` returns
   reference declarations such as `secret:callback-key`; resolve secrets in the
   adapter constructor and return an accurate `readiness()` state. Production
   adapters must not use the template's fixed fixture HMAC key.
2. Implement `MessageExtension::send` for the explicit mutation ID/context contract.
   Validate address and byte bounds before making an effect. Store the full original
   request alongside its logical ID. Same ID and same request returns the original
   result; same ID with changed payload fails. If the upstream has no idempotency
   API, do not simulate one with memory or resubmit an uncertain effect. Implement
   `message_status` for authoritative status reconciliation under the original
   mutation ID, preserving the first effect rather than sending again.
3. Return `Some(self)` from `Connector::message_extension` to opt into the host's
   normalized message path. `deliver_observed` converts the host's bounded typed
   projection and sends under the durable job ID. Override it when you need to
   retain additional host envelope fields, as Prism does. The host persists the
   entire receipt, actual reply source IDs and lifecycle projection with the reply
   in one transaction. A dispatch receipt must echo that exact durable job ID;
   a mismatched ID leaves the original attempt uncertain for reconciliation and
   cannot select another job. Existing `deliver_attempt`/`deliver` adapters retain their
   single-attempt compatibility path. The template rejects legacy `deliver`
   because it cannot invent a trustworthy ID.
4. Keep `DeliveryRequest::context` separate from `body`. Use the public API-only
   `host_context::HostContext` decoder and the [exact host projection schema](../contracts/connectors/host-context-v1.md).
   Ordinary conversation context is `ContextView`; task-origin context is the
   tagged `TaskOutcome::Recall` shape. They carry different omission/provenance
   metadata. Both are selected, authorized historical data. Preserve those sources, bound bytes
   before parsing, and identify actual sources in normalized `reply_sources`.
   Do not expand the selection, silently fall back to another person's history or
   put historical text into a privileged/system instruction. Derived summaries and
   incomplete transcripts remain labelled historical material. Do not infer derived
   material from arbitrary `kind` text: the shared mapping's exact known outcome
   categories and opaque-body rules are documented in the projection contract.
5. Implement the PCM lifecycle fixture independently of messages: one `open` attempt,
   explicit `reconcile`, idempotent paired `close`, bounded format-aware media
   exchange. An uncertain open may already have created a remote session. Cancel
   and peer-loss recovery must reconcile/close that effect rather than starting
   another one. Reject unsupported routes and codecs before starting media.
6. Authenticate callbacks over exact raw bytes before normalization. Bind logical
   and external IDs; never treat a phone number, provider call ID or message body
   as permission. Preserve terminal state across reordered callbacks. The public
   reducer checks current host authorization before duplicate handling; it does
   not replace the host's policy check or transactional journal.

## Declare real live media

Manifest v2 uses `MediaSupport::Live` for an actual host/transport contract.
`LifecycleFixture` describes the bounded PCM exercise, and `Unsupported` describes
message or telephone-control profiles with no media. There is no v1 fallback.
A changed persisted manifest fails closed and requires an explicit migration.

Use the existing rvoip `ConnectionAdapter` interface for a transport. Trusted
composition can call `connect_server::connector_media::validate_adapter` to
compare the declaration with the actual adapter's binding, codec families and
liveness/inbound/staging/replacement guarantees before registration. This check
makes no network effect and grants no route or operation. Keep Core registration,
current Store policy, negotiated media and shutdown with the host.

The canonical SIP startup path checks its real adapter and persists its contract.
Its codecs are negotiated per Connection; registration cannot invent PCMU, Opus
or video availability. Canonical Vapi hosting instead declares a signed native
Endpoint with PCMU, selected historical context and host-authorized recording;
its existing Endpoint/key/source checks and owned provider job remain required.
Chat, native hosting and telephone API control have distinct manifests.

The [v2 contract](../contracts/connectors/profile-v2.md) defines the closed media
union, static versus negotiated codecs, control declarations and context/record
policy. Registration and advertised capability remain observations, not evidence
of an answered call, decoded media, provider delivery or a human decision.

## Register through trusted composition

A host installs the adapter through the existing public composition boundary:

```rust,ignore
let driver = std::sync::Arc::new(MyConnector::from_configuration(config)?);
let worker = connect_server::providers::Worker::with_drivers(
    server.service.clone(), vec![driver],
)?;
```

The host validates the entire composition's manifest/configuration/readiness and
rejects duplicate registration before recovering/claiming jobs. It persists the
manifest before dispatch. Restarting with an identical contract is allowed;
changing it fails closed and needs an explicit migration and requalification.
Route configuration, conversation membership, revocation, context access and
SMS enrollment/freshness/STOP rules remain ordinary Connect host operations.
Adding an adapter requires no provider-name branch in core dispatch. A manifest
cannot register a route or grant an identity new rights.

For an uncertain effect, trusted composition explicitly calls
`Worker::reconcile(service.as_ref(), driver.as_ref(), original_job_id).await`.
This rechecks current route, conversation and credential authority before
calling `message_status`, then checks policy again when committing. Status is an
observation of the original effect, not permission for another task operation.
The ordinary task reply boundary decides current task assignment; an answer after
task cancellation or expiry stays unassigned communication with its source IDs. It never
resets the effect to pending or calls send. Reconciliation can admit the retained
answer and unblock later queued turns. Sources must belong to the exact original
selection; revoked credentials and forged source IDs fail even on replay.

For lifecycle callbacks, authenticate the exact raw bytes with your provider's
verifier, bind the original logical/external IDs, and call
`Worker::observe(service.as_ref(), driver.as_ref(), &verified_receipt)` with
`Verification::VerifiedCallback`. The host's durable reducer handles duplicates,
reordering and terminal conflicts in the same transaction as any answer. Caller
fields and external IDs still convey no authorization.

For independent provider contacts, authenticate the adapter response/callback and
construct `lifecycle::IncomingMessage`. Call
`Worker::receive(service.as_ref(), driver.as_ref(), configured_route_id, &message)`
from trusted composition. The existing route binds the principal; the host admits
one ordinary journal message transactionally under current route and conversation
authority. Matching logical replay returns the same message ID, conflicting replay
fails, and revocation is checked before duplicates. Subsequent context includes the
new journal source through the ordinary bounded selection. SMS ingress retains its
separate signature and STOP/START policy boundary. Delivery-only receipts/status
for already dispatched SMS remain recordable after STOP or enrollment expiry;
they cannot refresh consent, clear STOP or authorize another send. Reply-bearing
observations and all new dispatch still require the current SMS policy.

Trusted composition explicitly selects `voice::Worker::with_observed_driver`
for the normalized telephone lifecycle. This requires a valid manifest and uses
`VoiceReceipt`, preserving the
private `RemoteCall` control capability alongside the typed observation. The host
persists the reducer with call journal changes; status uses reconciliation evidence,
terminal state resists reordered observations, and creation is never repeated to
resolve uncertainty. Existing builtin startup and `voice::Worker::with_driver` retain the previous
carrier dispatch path; the typed path is separately fixture-qualified.
Physical status/cleanup of an already bound leg continues after task revocation;
this host observation does not grant permission for another start/end mutation.
`MediaExtension` remains the contribution fixture's separate bounded media contract,
not an automatic transport installation or a new native media route.

The server's existing `completed` contextual receipt remains distinct from an
external delivery guarantee. Telnyx retains SMS character limits, signed callback
handling and STOP/START policy. Vapi retains assistant context overrides, chat
correlation, voice/tool-origin bindings and explicit telephone/native adapters.
Their v1 message manifests do not advertise media. SIP/PSTN cancellation and
rvoip media ownership remain with their existing adapters.

## Run the reusable contribution harness

Implement `conformance::Fixture` beside your adapter. Fixture controls inject
uncertain message/open outcomes, failure after acceptance and peer loss. They must
operate on the actual adapter and retained mock-provider journal. `configuration()` returns the fixture adapter's declared secret references. `restart()`
reconstructs the adapter without clearing effect IDs; `effects()` counts real
fixture effects. Expose a binary accepting `--contract-report <fresh-state-file>`:

```rust,ignore
let mut adapter = MyFixture::open(state_file)?;
let report = connect_connector_api::conformance::run(&mut adapter).await;
println!("{}", serde_json::to_string(&report)?);
if !report.passed() { std::process::exit(1); }
```

Then run `scripts/check-connector-contract.py --manifest-path <your/Cargo.toml>`
from the Connect checkout. The script invokes your binary, requires every mandatory
check, rejects failed/absent/duplicate checks, links public API and contribution
source digests, and preserves build diagnostics locally. It does not accept an
externally supplied success report. Ship the report and reproducible command with
explicit unsupported video/live-provider cases. Mandatory cases cannot be skipped
to make the report pass.

The harness's `Fixture::independent_message` creates a new mock-provider incoming
contact without calling outbound send, retains it across restart, and uses it as
a source in a subsequent answer. Host tests separately admit that new contact on
the same configured conversation and verify its use in bounded history.

The harness exercises independently initiated messages, attributable context,
restart and mutation conflicts, uncertain creation without repeat, invalid/tampered
callback authentication, duplicate/reordered/terminal callbacks, revoked replay,
voice start/active, generated decoded duplex PCM, oversized media rejection,
uncertain cancellation, failure after acceptance, paired teardown and peer loss.
Host tests separately prove real authorization and durable Connect recovery.
Run intentionally broken fixtures to prove expected harness failures. Provider
fixture mappings are offline evidence; historical live paths and new live evidence
must be reported separately.

An independent contributor should use the exported public kit and their own fixture
implementation. Record source, package digests, commands, observed friction and
repairs. Author self-use is not the independent exercise. After any API or guide
repair, repeat the affected harness checks.

## Retained task and provider integration

These existing integration rules and source-scoped evidence remain applicable.
Gate 5 fixture qualification does not replace their historical live proofs.

### Task context and replies

For a task-origin turn, Connect reconstructs context from the original invocation
grant and the task's current authority. A History or ContextView permission allows
a bounded, source-backed task recall page, with an explicit omitted count and
continuation cursor. A Send-only grant supplies no history. Task turns never
inherit the route's previous provider chat ID; doing so would bypass the local
context ceiling through remote history. The turn still belongs to the same
durable Connect conversation. Its authorized history comes from Connect's
projection rather than an unrestricted remote continuation.

Keep those boundaries in Connect rather than rebuilding task policy in a
connector. Deliver the supplied context as conversation data, preserve source
references, and return the provider ID and answer through `DeliveryResponse`.
Connect derives the answer's task and question from the committed provider-job
effect link. Message admission, task inbox and wakeup commit in one transaction.
A late answer to a cancelled or expired task stays visible as unassigned
communication and does not resume the task.

The [provider task evidence](evidence/gate-4/provider-task-boundaries.json)
distinguishes store and public-client qualification from the remaining Vapi
MCP, voice and installed-host acceptance.

### Task-bound native voice context

The Vapi media adapter uses `Store::prepare_vapi_session_context` to select and
persist the exact context before attachment. Native sessions must be accepted,
linked to the task and currently permitted by the invocation. History comes
from bounded task recall under the grant's ceiling. A media-only invocation
supplies an empty source selection; that does not claim the conversation has no
history. The adapter checks the current route again after provider attachment,
before exporting context. Transcript and outcome observations remain in the
persistent conversation after the leg ends.

The [voice context evidence](evidence/gate-4/task-voice-context.json) records
qualified local SDK/media behavior and its limits. This path does not yet constitute
verified Vapi MCP installation or the full Gate 4 post-call/restart journey.

### Verify a Vapi origin before registering its task binding

`Vapi::verify_origin` uses the pinned private API SDK to match a call, standalone
chat or chat session against protected setup's exact resource and saved-assistant
IDs. Calls and sessions must be active; a session chat cannot masquerade as a
standalone chat. Invalid path identifiers never reach the provider. Lookup
failures return safe errors rather than raw provider bodies.

`VerifiedVapiOrigin` is a read-only observation with private fields. It is neither
an invocation grant nor a substitute for current task authority. Trusted setup
must still register and issue under current authorization, handle uncertain
creation, export only the invocation's credential and revalidate lifecycle.
The [origin verification evidence](evidence/gate-4/vapi-origin-verification.json)
qualifies that earlier lookup component. Protected first-invocation composition
is implemented in [Vapi task bootstrap](VAPI_TASK_BOOTSTRAP.md). Historical
[voice continuity](evidence/gate-4/live-voice-continuity.json) and fresh
[dc73996 scoped text/tool use](evidence/gate-4/actual-vapi-chat-dc73996.json)
retain their actual source and topology. Current owner-review repairs are qualified at 709cf82 in the [numbered plan](connect-gate-4-implementation.html).
The lookup checkpoint remains historical component evidence; the current full review is linked below.

### Register and issue for an existing verified resource

Protected Rust composition can call `connect_server::vapi_task_setup::issue`
with the backing credential reference, stable issuance ID, exact task and
provider origin, expected assistant, operation subset and original expiry.
The resource must already exist. Current authority is checked before the private
lookup and again inside the final binding/grant transaction. A failed lookup
cannot register an origin; a failed grant insert rolls back registration.
Matching replay preserves the secret and expiry. Call, standalone chat and
session chat have separate adapter namespaces.

The returned `VerifiedVapiTaskGrant` supports explicit protected export through
`issued()` and fixed MCP admission through `authenticator()`. Each MCP request
still rechecks current grant/task/parent authority before interpreting provider
correlation headers. Provider origins remain excluded from the public local
task-access exchange. Neither model arguments nor headers can invoke setup.

The [setup evidence](evidence/gate-4/vapi-task-setup.json) retains its historical component qualification.
This existing-resource API does not create provider resources, handle uncertain creation,
install tool configuration on an assistant, or solve first-invocation bootstrap.
Those lifecycle steps and main runtime integration are delivered separately through
[Vapi task bootstrap](VAPI_TASK_BOOTSTRAP.md); the current full review is linked below.

Historical Gate 4 owner-review qualification: Runtime 709cf82 passes 42 offline phases, 17 installed checks and 32 actual MCP calls with no errors. Qualification controls d64f1b9 combine the reviewed fixtures at 7bdd534 with all 3 hosted jobs. Local macOS arm64 owner review only; Gate 5, community publication and remote deployment remain separate.

[offline-709cf82](evidence/gate-4/offline-709cf82.json), [package-709cf82](evidence/gate-4/package-709cf82.json), [real-model-agent-709cf82](evidence/gate-4/real-model-agent-709cf82.json), [review-remediation-709cf82](evidence/gate-4/review-remediation-709cf82.json), [qualification-controls-d64f1b9](evidence/gate-4/qualification-controls-d64f1b9.json)

Current Gate 5 local technical qualification is recorded for the unpublished
`284cef1` candidate: [complete installed proof](evidence/gate-5/complete-installed-284cef1.json),
[actual-model evidence composition](evidence/gate-5/actual-model-composition-284cef1.json), and
[six bounded provider compatibility claims](evidence/gate-5/provider-claims-ab843b8.json).
Independent human walkthroughs and final documentation, quality, seal and owner
acceptance remain open. Historical provider receipts do not imply a fresh carrier,
Vapi or account-readiness check on this candidate.


## Independent contribution example

[Prism](../examples/connector-contribution/README.md) was written by a separate
implementer using only an exported public API/guide kit. Its original ledger,
message/media logic, boundary tests and deliberately broken modes are retained.
The host bridge now uses the shared typed projection decoder after the exercise
exposed incomplete schema guidance. This integration repair is distinguished from
the original independent authoring in the contributor evidence. The example
imports only `connect-connector-api`, and its own Cargo workspace builds without
changes to conversation, routing or authorization code.
