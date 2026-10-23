# Private provider context

G6-039 is complete for the installed canonical runtime and qualified local
native/designated-provider profiles. Schema 52 supplies the Store boundary for a private
provider instance and its content. Server composition now also supplies
`private_vapi::PrivateVapiBridge`, which owns a fresh signed native peer and an
actual private HTTP/WebSocket provider transport. Its local designated provider
fixtures are distinct from controlled Store probes. Neither qualifies a live
Vapi account or remote deployment. The
[completion audit](../evidence/gate-6/private-consultation-complete.json) separates
actual native/provider cases from controlled Store custody probes.

Schema 53 adds configured initialization from the ordinary signed or HTTP
`session.consult` operation. A protected host installs
`PrivateVapiCoordinator::install(&service, targets, secrets)` once, after the
canonical runtime is installed. Each `PrivateVapiTarget` selects one AI Identity,
its independently enrolled native peer, verified native/provider Endpoints and
explicit application/model profile. Participant commands cannot choose these
keys, prompts, endpoints or secrets. The coordinator always uses the original
caller's current grant for context export; it never substitutes an owner key.
The public authorized Conversation snapshot supplies the durable consultation
reference used by the existing typed return command.

For an absent configured AI, `join_configured_native_consultant` commits ordinary
Session invitation and acceptance receipts, their journal events and an immutable
parent binding in one transaction before native/provider IO. The caller needs
current `session.invite` and Session management authority. The configured AI key
separately needs `session.accept` and current recipient authority. Its later
native offer still requires its own current signer, Device and Endpoint. The
two child revisions are attributed to this exact original consultation; neither
the original request nor its digest is rewritten. Completion advances from that
effective revision once. An unrelated topology change still conflicts. This
installed join profile requires a voice Session with compatible PCMU support.

Owned jobs are bounded by the connection capacity. Replays share the original
job and cannot recreate an already registered durable candidate after eviction
or restart. Shutdown cancels and drains original job handles before retiring
the server's recording authority. A timeout retains unjoined handles for a later
drain; it does not detach replacement cleanup tasks. A failure before any
possible native/provider effect records only the exact admitted intention as
failed, including after execution-key revocation. Possible effects retain their
independent observations and uncertainty.

Cancellation records a decision immediately even while an already submitted
authorized creation/export can still return. The returned observation is captured;
it cannot authorize the next phase. A cancelled initialization with no native
audience attempt settles as `cancelled` only after any possible provider has an
actual termination observation. If creation's response is lost, the operation
stays `unknown`: neither cancellation nor replay may recreate it, export context
or claim the unobserved call has ended.

Designated failure fixtures withhold creation/export responses, interrupt a
creation response body, and refuse creation/termination receipt writes. They
exercise cancellation before response, late exact acknowledgments, metadata-only
recording recovery, no duplicate create/export/end and original call audio in
both directions. Endpoint revocation retires the private native source and closes
its exact provider; actual remote EOF then produces a return with
`remote_continuing: false`. Recorded failure is distinct from observed termination.

The AI may also be the operation's caller. It uses the owner's explicit native
Connection as the private primary, a fresh configured AI Connection, and its own
current context grant. The same AI's older public native Connection remains
excluded. The human primary owner independently authorizes return. No owner's
context key is substituted for the AI caller.

Before native completion, current authority checks recognize the parent-bound
invitation delta using one exact intent/digest/owner/runtime query. The compiled
SQL program is cached; current rows and permission fences are checked each time.
This avoids repeatedly loading effects and the journal on the audio path without
introducing an authorization-result cache. Strict positive audio thresholds and
negative isolation checks remain in place. A deliberately retired public peer is
not repeatedly polled as a live media source in the invitation fixture.

A target must be an explicitly configured, verified `vapi-voice` Endpoint owned
by the original private AI Participant. It needs `audio` and
`private-consultation` capabilities, a current Device and a server-owned secret
reference. Audio support alone does not select a private provider. The Endpoint
revision, address, secret reference and Device descriptor are pinned. The
context credential and native delegate independently require current
`context_view` and recipient ceilings for the original private audience.

`prepare_private_provider_context` consumes an exact current native consultation
planning read. It drops that reader before acquiring metadata writers. It freezes
the original physical selection, original two-participant audience, configured
target, bounded context JSON and one creation attempt. The context JSON includes
only sources both current principals may export. It is not returned as an idle
export grant. The context object holds metadata and weak installation ownership.

Initialization uses bounded queued metadata admission for creation and export
reads. The host retains its existing five-second preflight deadline, cancels the
wait on timeout or shutdown, and rechecks current credentials, exact selection,
original audience and exported content after waiting. Metadata admission does
not start a provider request. The immediate
`try_private_provider_read` API remains available, and packet callbacks keep their
existing nonblocking media reads. A contended packet path never queues behind a
writer. Creation/export begin tokens and actual transport custody remain separate,
so waiting for metadata cannot repeat a possibly submitted provider effect.

Creation and export have separate ordered custody:

1. Obtain `private_provider_read(context, wait)` on a blocking metadata worker
   and call `begin_private_provider_context`. This interval exposes the configured
   assistant and secret reference; `json()` refuses. The runtime must retain the
   bounded reader through one actual fresh provider creation. A saved public call
   or memory reference is not a fresh creation.
2. Drop creation readers, capture `PrivateProviderAttempt::created` from the
   actual creation result and record it. The Store checks existing public/private
   context deliveries, provider jobs and voice call references before reserving
   the provider reference. A conflict cannot expose private JSON or authorize
   closing the public call.
3. Obtain a fresh read. Only the recorded instance can now receive `json()`.
   `begin_private_provider_export` durably reserves one export to that exact
   reference. Keep the current read through the bounded provider write. The
   runtime must verify its owned actual call matches `provider_ref()` and send
   the sealed selection as untrusted history under application-owned prompts.
4. After actual acknowledgment, drop export readers and record the captured
   `PrivateProviderExport::delivered` result. This constructor cannot choose a
   different provider reference. Captured recording retries cannot send another
   body, create another call, attach media or revive a retired physical source.

Each reader is at most five seconds and clips to current source/context grants.
Creation/export deadlines are persisted before possible IO. Expired workers,
source loss, cancellation or uncertain IO cannot repeat the attempted phase.
Current writers wait for an outstanding export reader. Uncertainty can be
recorded while retaining the actual opaque attempt for a later captured result;
late observations remain recording-only. An uncertain root is not automatically
restarted after a late creation acknowledgment.

Generic context completion, transition workers, cleanup leases and operator
attestations cannot complete this private initialization. Once provider creation
may have reached the network, cancellation remains unknown until actual scoped
termination or other required settlement is known. `record_private_provider_closed`
and `record_private_provider_created_closed` are trusted **observations after
actual termination**, not remote hangup commands. The Store never performs those
network effects. A current separately authorized cancellation may settle a known
closed preparation when no audience effect remains uncertain.

AI media preparation and begin both require independent confirmed context
initialization. AI completion additionally requires the actual captured audience
application, decoded media and exact current native physical incarnations. Kind
labels, Endpoint verification and API/transport acknowledgment do not replace
these observations. Human completion keeps its own native readiness path. An
existing public Connection cannot be adopted as the private candidate.

Final transcripts use the exact captured provider instance and current physical
context. Sender, Conversation, Session and audience come from the original
binding; the callback cannot choose them. Repeated identical source IDs are
idempotent, while changed bodies conflict. Message and journal audiences pin the
original private membership epochs. Current authorization is checked again
before recording a new transcript; a historical initialization receipt is not a
future content grant.

`private_content_sources` preserves Connection/root provenance for private
transcripts and context deliveries. Ordinary provider conversation projections,
task-context exports and export revalidation exclude these sources. A matching
private context can select earlier private sources only when every original
target participant, at the current membership epoch, may read them. Projection
and export revalidation enforce the same audience rule for every selected source,
including ordinary messages. The original private audience is checked even when
the AI is both context owner and delegate. Revoking the human listener's history
share stops subsequent opaque-memory use while retaining the historical delivery
receipt. Logical identity access
for an authorized user's history does not automatically export that history into
an older public model context. Whole-Session provider context paths permanently
require a Connection-bound context once a private candidate has existed.

`PrivateVapiBridge::start` is a protected server library API. Its caller supplies
the accepted original transition, explicit execution/context credential IDs, a
configured AI native peer, the native/provider Endpoint IDs, a transport profile
and a server-only secret resolver. Both original private participants must already
be present in the main Session. `PrivateVapiCoordinator` supplies protected
profile selection and ordinary invitation/acceptance scheduling from an accepted
public consultation. Stock-listener/catalog/CLI composition is G6-044; no model
or unsigned HTTP request can invoke this trusted composition boundary.

The bridge registers an immutable fresh private offer, opens a signed public
`CanonicalClient`, negotiates one duplex PCMU stream, and requires the separately
queried signed durable `prepared_muted:true` result. Native offers end with their
actual answer; no extra final ACK is invented. The new `PrivateNativeAudio` SDK
type owns the peer that performed this negotiation. A Connection ID or copied
answer cannot construct it. The provider adapter is never attached over another
adapter's existing Connection.

The transport profile uses an explicit application prompt and configured OpenAI
model name. Saved assistant tools/tool IDs are replaced with empty lists; this
voice profile supplies no task or MCP tools. It creates a fresh `vapi.websocket`
call with mono Mu-Law at 8kHz and pins its returned reference before opening the
context socket. HTTP redirects are refused. WebSocket origin and
`/<captured-call-id>/transport` must match the configured origin and actual
reference. Plain HTTP/WS is limited to explicitly selected numeric loopback
fixtures. Diagnostics expose no secret, transcript or context body.

After one private `add-message`, the bridge requires a `conversation-update`
containing that exact system message. Queue admission, a completed socket write,
a status event or generated speech cannot acknowledge initialization. The reader
remains held through the write and acknowledgment; fractional wall-clock expiry
clips its actual deadline. Timeout drops the context socket before releasing the
reader, marks uncertainty and cannot repeat creation or context. An already
reserved exact call may receive a bounded cleanup attempt through a new empty
control socket; cleanup never replays private content. Only a captured terminal
status or confirmed closure records termination. A mismatched/public reference
never authorizes opening or ending its transport.

The installed bridge forwards20ms PCMU frames through the owned native peer,
with a bounded eight-frame queue that discards stale complete frames. Native
RTP timestamps follow elapsed monotonic time rather than packet counts. Silence
frames maintain the native stream through provider speech pauses; skipped ticks
cannot make later speech carry an old clock. Each current
provider media interval revalidates the initialized call, exact physical scene,
Endpoint snapshot, context/delegate credentials, audience and all sealed history
sources. Native candidate packet grants independently revalidate those facts and
clip to context-credential expiry, so an old queued packet cannot rely on the
original initialization grant. Checks batch original journal sequences and
validate each distinct resource once. Native readiness memoization requires the
same exact scene/audience, SQLite data/schema versions, own-connection change
counter and integer policy-clock second. Every relevant own/foreign write, schema
change, expiry boundary or changed argument invalidates it. It holds no body,
credential, reader or grant; native reads still acquire and verify current fences
around lookup. New transcript writes perform the same current history check. Losing a
history recipient ceiling closes candidate eligibility while preserving the
primary's own native Connection for an independently authorized return.

Captured creation/export/termination observations retry metadata without repeating
network effects. Blocking Store and secret resolution work runs outside the async
transport loop. Silence/idle datagram timeouts do not end the call. A failed
provider media write drops its socket without flushing queued private bytes after
the current grant. Provider terminal events are observations, not requests to send
another end command.

The actual local fixture matrix checks fresh creation, once-only acknowledged
context, both audio directions, held-remote isolation in both directions, an older
public Connection of the same AI Identity, private transcript/history and snapshot
visibility, current context revocation, observed cleanup and authorized return to
the same main call. Missing or mismatched acknowledgment preserves public audio
and cannot repeat creation/export. A provider-ended event records observed
termination without a second end command. A dropped socket leaves termination
unconfirmed; a fresh authorized return still restores the original call. These
seven scenarios use a human primary and a fresh AI candidate. Refusing native
begin after actual context initialization cancels a durable zero-attempt effect,
releases only the provisional primary floor and restores original audio in both
directions after scoped provider cleanup. The socket-loss scenario additionally
holds a real SQLite writer during private forwarding, then verifies recovery
without replaying speech queued before the stall.

Packet diagnostics identified repeated SQL preparation in current native policy
checks. Hot queries now reuse compiled statements, while rebinding parameters,
reading current rows and acquiring the same authority fences on every call.
These compiled programs do not cache authorization results.

Stopped native preparation has separate recording-only custody. Settlement
drains packet readers with a Conversation write fence, validates the durable
unstarted attempt and cancels only its prepared effect. Primary publication is
blocked while that provisional floor is unsubmitted, preventing queued private
input from being reclassified as public when the floor is removed. The candidate
remains private. A captured preparation cannot cancel a possible native submission.
Revoked execution credentials do not prevent narrow no-IO metadata settlement.
Provider cleanup remains independently unknown until actual termination is
recorded, including lease-expiry recovery.

Provider PCM is buffered for at most 200 ms from the earliest received fragment,
within 1280 bytes. Complete and partial expired frames are discarded; subsequent
fresh speech is preserved. This bounds local replay latency across a metadata
stall without changing current audience authorization. A controlled reproducer
identified delayed provider speech by its decoded amplitude; the strict
remote-only silence assertion remains unchanged.

The current local two-thread checkpoint passes 665 library tests and 100
runtime/SDK/CLI/transition tests, including all twenty-one designated provider
cases. Thirteen private-provider marker pairs exceed unchanged thresholds.
The additional six-peer two-group case verifies fifteen strict audio probes,
signed snapshot isolation, separate returns and reusable original routes.
Six controlled receiver-role overlaps refuse before a competing intention.
The new candidate lookup ignores unrelated completed/revoked execution sources
while requiring its own current original parent and exact native incarnation.
Exact source/binary digests, earlier failed attempts and the requirement audit are
in [the completion evidence](../evidence/gate-6/private-consultation-complete.json).
The earlier [primary recovery evidence](../evidence/gate-6/private-primary-recovery.json)
remains historical evidence and retains an unexplained earlier Session end replay
timeout. No general load, timeout/performance SLA, live-model or external-profile
qualification is inferred.

G6-039 is complete for this installed behavior. G6-040 handoff is next.
General provider integration belongs to G6-044; general context and observation
expansion belongs to G6-042/G6-043. Historyless and tool-enabled profiles need
explicit policy and are unsupported by this history-aware voice profile.
The explicit server library API requires protected host installation. Default
verification uses isolated local designated fixtures. Named remote/provider
rehearsals require G6-093's bounded configuration.

Protocol references: [Vapi WebSocket transport](https://docs.vapi.ai/calls/websocket-transport)
and [conversation-update events](https://docs.vapi.ai/server-url/events).

## Final transcript scheduling (G6-044 in progress)

The private media loop enqueues qualified final transcripts instead of waiting
inline for journal writes. The ordered queue holds at most 32 callbacks and
1 MiB of source/role/body bytes, with one Store attempt in flight. Source IDs are
retained across busy retries. Missing text and unsupported speaker roles remain
optional observation data and do not become archived messages.

The server uses `Store::try_record_private_provider_transcript`. It performs one
immediate authority and SQL attempt, with the same original context, provider
receipt, current memory, audience, epoch and sender checks as ordinary transcript
recording. It does not wait on the process SQL mutex, another SQLite writer or
an authority writer queue. Busy results leave the callback queued for retry; no
provider request is repeated. Media still requires its own current read interval.

When forwarding ends, observation ownership closes synchronously and media reads
are released before archive cleanup. Cleanup joins an admitted Store attempt.
Cancellation starts no further queued callbacks. Normal draining has a five-second
retry budget plus completion of an already admitted bounded Store attempt.
Overflow returns `private-transcript-capacity`; a cancelled or timed-out pending
archive returns `private-transcript-unconfirmed`. Uncommitted callbacks are not
reported as successful transcript import. Queued entries are process-local and
are not a crash-recovery or lossless transcript guarantee.

This removes inline archival waits and queued writer/SQL contention from the
media handler. The earlier one-frame duplex failure is still an observed
unresolved incident, not an established diagnosis or availability qualification.
Real-provider proof must rerun private Vapi voice cases with final callbacks
before/through audio, contention, cancellation, callback duplication and teardown,
using the concrete bounded endpoints/configuration required by G6-093. Default
checks use isolated native and loopback provider fixtures.


## G6-046 native refresh regression checkpoint

The schema86 native credential-epoch continuation preserves the current provider
path. All eight original local private-provider/record/cancel/retention-return
cases and three native/owned DTMF regressions pass with unchanged bounds. The
three new automatic native refresh cases also pass for EdDSA and ES256, including
independently decoded RTP after predecessor expiry. See the
[scoped evidence](../evidence/gate-6/native-credential-refresh.json).

The 36 serial private-provider Store checks pass. The earlier default-parallel
29/36 result remains unqualified, and the historical primary-return refusal
predicate remains unestablished. These selected passes do not establish general
parallel/return reliability, named external-provider profiles or full G6-046.
