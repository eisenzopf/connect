# Assistant context across Sessions

G6-042 is in progress. Context selection and a bounded current read are separate
from provider creation, actual delivery, acknowledgment, and media readiness.
A context preview must never be reported as a successful provider delivery.

`ContextView` retains attributed messages, file metadata, and Session outcomes.
Canonical `session_control` items contain the Session snapshot at that journal
position. Ordinary Session and artifact events also retain their saved snapshots:
a later accept/end or completed upload does not rewrite an earlier event. Current
source permission is checked independently. File bytes require the artifact APIs;
context contains no server file path or implicit download capability. Task recall
has its separately documented current-metadata semantics.

For a registered native assistant, protected server composition calls
`Service::try_assistant_context_read(context_credential, connection_id,
generation, provider_endpoint)`. The public Connection ID and exact generation
select the actual registered native incarnation. The service checks its live
adapter and activation; the Store resolves its real origin, current AI
Participant/epoch, Session presence and verified provider Endpoint. It does not
infer a destination from the first Session invitee.

The context credential and native assistant credential independently need current
`context_view` permission and recipient ceilings. Every current public Session
listener must be authorized for every selected source. Thus history visible only
to the owner and AI does not enter a public Session with another listener.
Filtering happens before the item limit, watermark and omitted count. The source
and configured verified `vapi-voice` provider Endpoint must belong to the same current AI identity;
the native endpoint itself cannot substitute for a provider Endpoint.

The resulting `AssistantContextRead` lasts at most five seconds, clipped to the
original current grants. It retains one complete authorization interval, including
the context owner, actual native source and current listeners. Queued revocation
waits for that interval; subsequent reads refuse after revocation. The service
also checks physical liveness and activation on use. Provider IO must retain the
read, check `current()` through each bounded IO poll and abort when it is false.
Copying JSON does not copy authorization. Release the read before recording
observations or acquiring metadata writers. The read does not reserve an effect,
create a call, or acknowledge context delivery.

Private physical destinations require the existing Connection-bound private
provider purpose. The public reader refuses them. Private context selection and
opaque-memory revalidation require every original private listener to retain
access to every exported source, including ordinary messages. When the AI is both
context owner and delegate, its own permission does not substitute for the human
listener's permission. Revoking a source's explicit history share stops further
use of that opaque provider memory while preserving the recorded delivery.

The protected reader is a server library interface, not a model tool or a new HTTP
command. Public communication continues through the
[canonical HTTP interface](canonical-http.md), with the
[host context envelope](../../contracts/connectors/host-context-v1.md) defining
attribution, bounded selections and receipts. The captured public provider
transport below supplies creation and exact context export for the registered
native destination. Its optional owned public audio composition is described below.
Public voice context supports initialization-time delivery and updates to an
existing owned instance, as described below. The full restart journey remains open. Local fixtures qualify their
recorded macOS scope; Linux, browser, remote and live-provider profiles require
their own later verification.

## Captured public provider instance

Schema 63 reserves each public voice-provider request by context owner and stable
operation ID. `Service::prepare_assistant_provider` consumes the actual native
reader, releases its read interval before acquiring metadata writers, and resolves
current authority again. The reservation fixes the original credential, native
Connection/generation, verified provider Endpoint, current listener epochs and
exact historical selection. A repeated operation cannot select a new physical
call or use a replacement credential to create a second effect.

`try_assistant_provider_read` and `try_assistant_memory_read` revalidate the
original selection at its saved watermark, its whole audience and the actual
registered adapter/activation. Newer messages do not rewrite that selection.
The creation and export boundaries consume independently current readers and
reserve one attempt before IO. Raw context is unavailable to the creation reader
until its actual provider reference has been captured in the post-creation mode.
The export token accepts
only the exact echoed context body. A socket write or status update is insufficient.
Captured observations can be recorded after credential revocation without granting
another export. Both public and private creation observers reject references
already captured by conversation deliveries, provider jobs, voice calls, or either
native provider lane. A reused response ID remains uncertain for the new attempt;
it authorizes neither export nor cleanup of the foreign instance. Definite
rejection and observed truncation record failure. A
provider ending before exact acknowledgment records uncertain delivery with an
explicit incomplete problem; its observed terminal state authorizes no repeated
end command. Other unknown creation/export outcomes remain uncertain.

`AssistantVapiContextSession` supplies an owned HTTP/WebSocket context transport
for this binding. Its application/model profile and secret resolver are protected
host configuration. Creation installs the application prompt with no tools; the
separate historical system message is untrusted data. Transport addresses must
match the captured reference and configured origin. Redirects are refused; plain
HTTP/WS requires an explicit numeric loopback fixture. Received context frames are
bounded to 512KiB, allowing the bounded JSON selection's escaped wire envelope.
The host retains and drains the owned job; cancellation does not authorize a new
creation or export. Cleanup reserves one end exchange for the captured call and
preserves uncertainty when no actual terminal exchange is observed. Receipt and
lifecycle observations are journaled to their original listener epochs.

`start` supplies context control only. `start_with_audio` additionally consumes
the `PublicNativeAudio` returned by the public SDK's own signed offer, answer and
durable operation confirmation. The SDK rejects private/handoff markers and
requires an ordinary fresh admission. Server composition compares its Conversation,
Session, Connection and original generation to the protected context destination
before provider creation. No constructor adopts another peer from its ID or answer.
Before transferring ownership, the host selects audible sources with the SDK's
signed `subscribe_audio` method. It preserves the current stream revision,
validates any actual reoffer as one PCMU stream, and requires the durable completed
selection and command ACK. A failed/uncertain selection closes the peer. This
fixed profile does not silently accept codec or multi-stream changes. Connections
belonging to the same Participant are subject to the mixer's normal self-exclusion;
two physical peers of one AI are not an independent duplex audience.
Persist the original selection operation ID, expected revision and selectors;
replay sends that same intent even after the local current revision advances.

The owned job begins PCMU forwarding only after exact context acknowledgment.
Every provider/native audio poll requires a current original-memory reader and
actual live native activation. Metadata contention drops forwarding authority
until a new complete read succeeds. The shared queue retains at most1280 bytes
and expires speech after200ms; a delayed writer cannot replay an old queue.
Context revocation stops forwarding, cleans up the captured provider and closes
and drains the job's owned native peer. `ContextReady` still describes the exact
context acknowledgment; only observed duplex audio proves media behavior.

The complete public-call/private-consultation/restart-follow-up journey remains
separate implementation and qualification work.
The ordinary chat initialization fixture does not qualify these voice paths;
voice initialization and update checks use their actual owned HTTP/WS job.

The public `ContextDelivery` read includes the captured native destination,
problem and control lifecycle through the existing named `ApplicationClient`
context APIs. A retired native Connection or changed generation cannot rewrite
that original destination. Private receipts use their original two-party resource
audience; broader Conversation membership is insufficient. Cleanup admission is
nullable for lanes without a separately recorded phase. These are historical and
current control observations, not media readiness or new provider authority.

## Initialization-time voice context

The protected `AssistantVapiProfile::context_timing` selects `AfterCreation` or
`Initialization`. This choice is immutable for a logical creation operation.
Initialization admits one compound creation/context export before the HTTP
request: the original attributed JSON enters `assistantOverrides.model.messages`
alongside the application prompt, with no tools. The bounded current read spans
that request. A lost response therefore leaves both a possible creation and a
possible context export; a null provider reference never means that no history
left the process. The receipt reports `phase: initialization` and the admitted
export even before creation is confirmed.

After the actual reference is captured and checked for freshness, the owned
socket observes the exact initial system message. It does not send an additional
`add-message`. Opening a fresh acknowledgment reader cannot extend the original
inline export deadline. HTTP success, a returned ID, or opening a socket is not
context acknowledgment. No native/provider audio is forwarded before that ACK.

The configured create protocol must allocate a fresh instance. A reused response
ID is detected after the inline request may already have exported history: the
new receipt stays uncertain and authorizes no attachment or cleanup of the
foreign instance. This cannot be described as a zero-export refusal. Unknown
creation is never retried under the same logical operation.

## Updating an owned voice instance

`AssistantVapiContextSession::update_context(operation_id)` queues a bounded
host request on that job; it accepts no provider reference or model-selected
credential. Updates use the same owned socket and captured instance. Each has a
separate immutable `ContextDelivery`, original operation/attempt, watermark and
selection. Replaying its operation returns the original delivered receipt without
another creation or export. New history requires a new logical update ID.

The instance retains its initial selection and every earlier exported update.
Current memory validation therefore checks all those frozen selections under one
complete read interval, including sources no longer in the latest bounded page.
It does not acquire a second reader behind a queued writer. The original native
generation, credential, endpoint and listener epochs must still match. At most
sixteen update records belong to an instance; reaching that bound requires an
explicit fresh instance rather than discarding old memory authorization.

The audio job releases its current reader and queue before preparing the update.
Forwarding pauses until the exact new body is acknowledged and recorded, then
requires a new complete memory read. Truncation is failed; missing acknowledgment
or an interrupted exchange is uncertain. An observed provider end before ACK is
uncertain/incomplete and does not send another end command. A possible unconfirmed
export blocks further memory use and stops the job, preserving captured cleanup.
A later exact observation can record what happened after grant loss without
granting more IO or reopening an uncertain instance.

Lifecycle `phase` distinguishes `initialization`, `post_creation` and `update`.
An update's `parent_delivery_id` identifies the original instance receipt.
`creation_admitted: false` with `creation_confirmed: true` on that child describes
reuse of the captured instance, not a new create attempt. Cleanup fields belong
to that whole instance; their current values can change without changing a
child's original delivery timestamp or selected history. Recovery expires unused
updates as failed/incomplete and attempted unconfirmed updates as uncertain. It
performs no provider IO and never reconstructs an old capability after restart.
