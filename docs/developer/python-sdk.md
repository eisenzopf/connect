# Python canonical SDK

The installed wheel now also passes real local EdDSA/ES256 handoff and Endpoint
replacement controls with independent decoded native media, original-reference
recovery, eight helpers and four servers joined. The consumer uses the default
`connect` lookup in its isolated installation. See [scoped movement evidence](../evidence/gate-6/python-installed-movements-local.json). All 194 component
checks and the shared Node launcher regressions pass. Linux selection is wired
but not yet qualified; this does not close G6-063 or claim Python media ownership.

`connect_sdk.Client` is the asynchronous participant SDK. It owns one lazily
started `connect sdk-stdio` process using `connect-sdk-helper/4`. The public Rust
SDK still owns authentication, current authorization, the journal and operation
execution. The Python layer validates typed observations and original intent.
The historical `NativeClient` application interface is not a fallback.

The current control slice implements Conversation create/refresh, participant
add/update/remove/refresh, Session lifecycle and native Connection/Stream controls,
message send/receipts and typed original-ID operation
status, completion, readiness, cancellation, reconciliation and recovery.
Mutation await returns an `Operation[T]`; `wait()` returns `Completion[T]`.
`wait_until_ready()` reports readiness without asserting completion. Interrupted,
unknown and failed status values remain observations and never trigger replay.

```python
from connect_sdk import (
    Client, IdentityId, MessageId, MutationOptions, NonEmpty,
    OperationId, RequestOptions,
)

async def send(config: str, recipient_identity: IdentityId):
    options = RequestOptions(timeout_ms=3000)
    async with Client(config) as client:
        created = await client.conversations.create(
            [recipient_identity], "persistent",
            MutationOptions(OperationId("create-original"), options),
        )
        room = (await created.wait(options)).value
        snapshot = await room.refresh(options)
        recipient = next(
            p for p in snapshot.participants
            if p.identity_id == recipient_identity and p.active
        )
        operation = await room.send(
            MessageId("message-original"), NonEmpty(recipient.id), "Hello",
            MutationOptions(OperationId("send-original"), options),
        )
        # Save this JSON as private application state before waiting further.
        saved = operation.reference_json()
        result = await operation.wait(options)
        return saved, result
```

On a fresh explicitly constructed client, restore saved metadata with
`client.operations.restore(saved, client.operation_kinds.message_send)`, then
call `status(options)` before choosing any new action. `recover(reference)` uses
an SDK-created typed reference. Both forms construct metadata locally without
starting a process or granting permission. Every later read or decision is
authorized by the server. A new cancellation/reconciliation decision needs its
own operation ID; its receipt identifies the original target separately.
Serialized JSON is an independent mutable copy. SDK references, handles,
options, typed results and nested observations are immutable.

Python 3.11 and newer use standard-library asyncio subprocesses. Call a client
from one event loop. Use `async with` or explicit `await client.close(options)`;
close owns the original process and pending waits. Unsent cancellation/deadline
does not submit a mutation; a submitted timeout/cancellation retains the original
key and uncertain outcome, fences that process, and never starts another owner.
Normal `Task.cancel()` remains asyncio cancellation and carries `SdkCancelledError`
kind/certainty/original-operation metadata. Explicit cancellation signals use
`Cancellation` in `RequestOptions`. Default exception text/repr omits payload,
credentials, helper stderr, paths and cancellation messages. Application logging
must separately choose whether to export private observation/reference fields.

The synchronous API remains G6-063 implementation work. Its intended owner is a
dedicated event-loop thread with the same bounded dispatch, deadlines and single
process lifetime; calling `asyncio.run()` independently for each method cannot
share this client safely. Current public APIs are asynchronous. Remaining work
also includes HTTP parity, helper
packaging and the clean installed-venv real-server/restart qualification.
Capabilities advertise only Python interfaces currently exposed and supported
by the original helper. They do not grant server permission or native media.

[Component evidence](../evidence/gate-6/python-sdk-public-control.json) records
69 local fixture functions, including 16 public control and 17 owned helper
functions, plus the real public-type outline and six unsuppressed negative type
constraints. These synthetic child tests do not qualify an installed Python
package, actual native transport, remote server, full SDK or Gate 6 acceptance.
The earlier [Linux foundation matrix](../evidence/gate-6/python-sdk-helper-linux.json)
qualifies only its recorded source on Python 3.11–3.14; this expanded control
slice still requires its own Linux run.

Session controls include initial solo or addressed invitation, nonempty extension
of an existing Session, accept/reject/cancel/end and the four typed update
variants. Session state revision and Session topology revision remain distinct;
mutations use `TopologyRevision`. Rejection/cancellation returns a
`SessionDecision`, while end/update/accept completion returns a Session handle.

Connection control includes offer, quality/gates/codecs, end, Stream selection
and scoped Connection/Stream refresh. `StreamRevision` authorizes negotiated
stream controls; `StreamResourceRevision` is a separate resource observation.
`receipt_stream_revision` is historical and is absent on a by-ID handle. Null
selection bindings mean unavailable capture; an empty tuple means captured
empty bindings. Nested codec/format/clock data stays immutable. Source-clock
units retain the exact unsigned64-bit value. These handles provide control and
observations; Python programmatic media remains G6-064 work.

[The expanded control evidence](../evidence/gate-6/python-sdk-native-control.json)
records 88 local runtime functions, including eight Session and eleven
Connection/Stream functions, all five runtime modules and the public outline
passing mypy, and twelve unsuppressed negative constraints. The first combined
attempts failed because the Mac exhausted disk space; they are not passes.
Only reproducible Python test caches were removed. The successful rerun retains
raw logs privately. Linux and actual installed native/server qualification for
this expanded source remain pending.

Consultation, return, handoff and device replacement now complete the22 frozen
participant operation-kind mappings. `Session.consult(ConsultInput(...), options)`
returns a `ConsultationOperation`; its typed `return_to_retained(topology, options)`
creates a separately identified return operation. A saved consultation retains
its original host record. A previously unbound restored reference binds that
record on observation and exports it for later recovery. A different record
fences the original owner rather than silently rebinding it.

`Session.handoff(HandoffInput(...), options)` preserves the original retained
Connection, optional exact candidate, leave/observe disposition and Session or
Conversation role scope. An optional consultation uses the original SDK-created
reference. `Connection.replace_device(MoveInput(...), options)` accepts exactly
one `EndpointTarget` or `CandidateTarget`. Typed results preserve disjoint
retained/admitted/retired topology, revision families and captured role/binding
observations. None of these recovered handles acquires native media custody.
The actual backend candidate-preparation defect remains unresolved; synthetic
Python candidate tests do not qualify that production path.

[Transition evidence](../evidence/gate-6/python-sdk-transition-control.json)
records100 local functions, including12 new transition functions and the
47 public-control,17 owned-helper and36 historical regression functions
in total (the47 include these12). All six runtime modules and the actual public
outline pass mypy;18 unsuppressed negative constraints reject mixed IDs,
revision/result families, ambiguous targets and unsupported media custody.
Separate runtime cases reject foreign command-kind witnesses, including equality
spoofing. The22 Python kind names match the frozen contract and original
Rust helper enum. Complete installed/native/server/Linux qualification, events,
raw/HTTP/admin interfaces, synchronous ownership and deployment remain open.

The [exact6d Linux component](../evidence/gate-6/python-sdk-public-linux-6d.json)
passes69 functions and six negative constraints on Python3.11–3.14 with58 tracked
file contents/modes verified per artifact. It qualifies that older control slice;
the expanded88/100-function sources require their own Linux evidence.

The expanded [exactca3 Linux component](../evidence/gate-6/python-sdk-linux-ca3.json)
passes100 runtime functions and18 negative constraints on Python3.11–3.14.
All65 tracked file contents/modes match each retained artifact and the guarded
Python source at checkpoint01699bf7. This qualifies the synthetic participant-control/helper/type
component, including all22 mapped kinds; it does not qualify the original
server/installed package, actual candidate movement or remaining SDK interfaces.

Durable journal facts and incoming invitation hints now have owned asynchronous
readers. `room.events(after, options)` accepts an opaque saved `Cursor` or `None`;
`await events.ready()` finishes the original authorized bootstrap and returns its
baseline without consuming a fact. Each delivered `JournalEvent` carries its own
cursor, source, audience and original operation ID. Apply the event idempotently
by `event_id`, then persist its cursor as private application state.

```python
async with room.events(saved_cursor, options) as events:
    await events.ready()
    async for event in events:
        await apply_and_persist(event)  # Application-owned durable processing.
        break

async with client.incoming_invitations(options) as invitations:
    hint = await anext(invitations)
    current = await hint.session.refresh(options)
    # Make any acceptance/rejection as a separate explicit mutation.
```

Use `async with`, `contextlib.aclosing`, or `await reader.close()` when leaving a
reader. Python's `async for` does not automatically close an arbitrary iterator
on break. One waiter is allowed per reader, one invitation reader per helper,
and at most eight journal readers with one source per Conversation. The Rust
owner retains its source limits and exact acknowledgement protocol. Structured
source protocol/resync errors remain explicit and do not grant replay authority.

A local wait timeout, explicit signal or `Task.cancel()` consumes no fact or hint.
The original bounded poll stays owned. A later wait re-polls the Rust source
under current health before returning a retained result; it never delivers a
Python-cached result after revocation. Only values returned to the application
are acknowledged on the following poll (or invitation close). An unconsumed
invitation remains available to a fresh reader. Client close joins reader RPCs
and the original helper. Nested event and hint data stays immutable, and default
representations omit payloads. Invitations are hints; the journal is authoritative.

[Reader component evidence](../evidence/gate-6/python-sdk-readers.json) records120
local runtime functions, including20 reader functions,69 unchanged guarded inputs,
all nine runtime modules plus the public type outline passing mypy, and22 expected
negative type constraints. The first combined attempt exhausted disk before its
summary/type verification completed; its raw logs are retained as a failed
aggregate. The passing check disables disposable bytecode/type caches. Expanded
Linux, installed original-server/restart/current-authority and complete SDK
qualification remain pending; G6-063 stays in progress.

`client.raw` exposes finite canonical exchanges through the same Rust peer.
`validate(value)` synchronously invokes the frozen Rust schema/intent validator,
without a configuration argument, credentials or network. It returns an
SDK-created `ValidatedEnvelope` with immutable `value`; `to_json()` exports an
independent copy for private persistence. Arbitrary dictionaries and forged
lookalikes cannot be submitted as validated tokens. Local validation confers no
authentication or authority. The original Rust peer verifies returned signatures.

```python
checked = client.raw.validate({
    "v": 2, "id": "saved-local-attempt", "ts": "2026-10-28T00:00:00Z",
    "type": "operation.get", "cid": "saved-conversation",
    "payload": {"operation_id": "saved-original-mutation"},
})
replies = await client.raw.request(checked, options)
for reply in replies:
    signed_value = reply.value
```

The local validator owns one child, a two-second absolute validation deadline,
16-KiB output limit and a joined kill path. No private stderr is surfaced. The
Rust descriptor classifies mutation intent and finite-lifecycle support; Python
does not maintain a competing command-policy list. Raw execution retains payload
extensions and original logical operation IDs while Rust creates fresh physical
request metadata. Batches preserve every correlated frame and final ACK, signed
metadata and optional payload fields. Python rechecks frame identity, scopes,
terminal ordering and descriptor capture without claiming its own cryptographic
verification. A malformed batch fences the original helper; submitted mutations
remain uncertain and retain their original key. No automatic retry occurs.

Finite journal replay needs explicit `live: False`; live leases, authentication,
step-up, native offers/answers and binary transfer keep their dedicated lifecycle
APIs. The original one-MiB per-envelope/depth32/node100000 limits apply without
enlarging ordinary typed writes. Envelope and reply representations omit private
payloads. Local validation is synchronous and can briefly block its calling
thread, including when called from an async application.

[Raw component evidence](../evidence/gate-6/python-sdk-raw.json) records139 local
functions, including19 raw functions,72 unchanged source guards,11 runtime modules
plus the public outline passing mypy and25 expected negative type constraints.
All109 Node component/type regressions also pass after the shared synthetic
fixture additions. These tests simulate the frozen validator and signed replies;
they do not execute the original Rust validator, authenticate actual server
traffic, or qualify complete installed SDK behavior. HTTP/admin, synchronous
ownership, helper deployment and the installed original-server/restart/current-
authority journey remain required.

The [exact38 Linux reader component](../evidence/gate-6/python-sdk-readers-linux-38.json)
passes120 functions and22 type constraints on Python3.11.17/3.12.15/3.13.17/3.14.9.
Each artifact matches69 tracked file contents and modes and includes all20 reader
functions. This qualifies that recorded reader/control/helper source. The newer
139-function raw interface changes guarded sources and needs its own Linux proof;
neither component qualifies the full installed original-server journey.

`AdministrativeClient` is the explicit trusted backend facade. It owns
`connect sdk-admin-stdio` with the separate `connect-sdk-administration/2` profile.
It exposes no Conversation or raw participant interface; `Client` exposes no
provisioning method. The Rust owner uses the explicit configured application and
native endpoints with the same supplied credential and current server authority.
Python adds no route or credential fallback.

```python
from connect_sdk import AdministrativeClient, MutationOptions, OperationId

async with AdministrativeClient(admin_config) as admin:
    command = admin.validate_provisioning({
        "type": "identity", "operation_id": "provision-original",
        "alias": "local", "kind": "human", "display_name": "Local",
    })
    saved_reference = command.reference_json()
    private_intent = command.to_json()  # Persist privately before submission.
    operation = await admin.provision(
        command, MutationOptions(OperationId("provision-original"), options),
    )
    result = await operation.wait(options)
```

`validate_provisioning()` uses the original frozen Rust parser and normalizer
locally, without a configuration or network argument. Its immutable
`TrustedProvisioningCommand` preserves the original logical ID and normalized
intent. Commands and saved references grant no authority. Submission needs the
same ID in `MutationOptions`; nonempty provisioning extensions are unsupported.
Only SDK-created command tokens can be submitted. The validator is synchronous,
bounded to two seconds with128-KiB output and joined cleanup. Command input stays
at64KiB, reference input at4KiB, and the helper wrapper has its own bounded space.
Ordinary participant write limits are unchanged.

Administrative mutations return the same typed `Operation[Json]` interface for
status, readiness and completion. The original Rust kind validates the
provisioning outcome; Python exposes its immutable JSON. Cancellation and
reconciliation create distinct `Operation[DecisionReceipt]` decisions and retain
their original target. Stable record, key, scope and submitted uncertainty fences
apply. No automatic retry or configuration reload occurs.

On an explicitly constructed administrative client,
`admin.operations.restore(saved_reference)` validates the reference locally,
without submission, then returns `Operation[Json | DecisionReceipt]` according to
its recorded kind. `recover(reference)` retains the original typed result and
uses SDK-created administrative metadata without another validator process.
Every later read or decision rechecks current server authority. Direct typed
recovery rejects references from the other facade. JSON export remains explicit;
default representations omit private commands, configuration paths and payloads.

[Administrative component evidence](../evidence/gate-6/python-sdk-administration.json)
records156 local functions, including17 administrative functions,74 unchanged
source guards,12 runtime modules plus the public outline passing mypy and30
expected negative type constraints. The initial administrative run failed its
full64-KiB carrier test because the synthetic Node validator exited before
flushing its65757-byte reply. Its writer now awaits completion; the exact same
65536-byte input produces a complete reply. This fixes a fixture, without
relaxing the input limit. All109 Node component/type regressions pass afterward.
These checks simulate the validator and backend. Actual administrative
server/current-authority and installed-package execution still need evidence;
HTTP parity, synchronous ownership, deployment and the complete clean-venv
restart journey remain required for G6-063.

The latest [exact6c Linux component](../evidence/gate-6/python-sdk-administration-linux-6c.json)
passes all156 functions and30 negative type constraints on
Python3.11.17/3.12.15/3.13.17/3.14.9. Each artifact matches74 tracked file contents
and modes, and those guarded files match the recorded administrative source before the HTTP additions below. This covers
the control, reader, raw and administrative component, including the corrected
64-KiB validator carrier and original child cleanup. It resolves the earlier
pending Linux component check for this source. The original Rust/server,
installed-package/restart/current-authority and complete G6-063 scopes remain
unqualified.


`CanonicalHttpClient` provides explicit finite canonical HTTP control. It posts
current `connect/2` envelopes to `/v2/commands`; its policy matches the finite
HTTP command surface, including `conversation.subscribe` with `live: false`.
Choose `loopback-http` for a loopback HTTP origin or explicitly choose
`remote-https` for HTTPS. HTTPS uses the normal platform CA chain and hostname
verification. An explicitly constructed client keeps its captured origin and
credential; redirects, proxies, alternate routes and automatic retries are
unsupported. The application `HttpClient` remains a separate interface.

```python
from connect_sdk import (
    CanonicalHttpClient, ConversationId, HttpCommandScope, OperationId,
    RequestOptions,
)

async with CanonicalHttpClient("https://connect.example", credential,
                               transport="remote-https") as http:
    frames = await http.request(
        "message.send", {"msg_id": "message-original", "to": ["participant-original"],
                         "content_type": "text/plain", "body": "Hello",
                         "body_encoding": "utf8", "label": "connect-messages",
                         "attachments": [], "in_reply_to_msg": None},
        HttpCommandScope(conversation_id=ConversationId("conversation-original"),
                         operation_id=OperationId("send-original")),
        RequestOptions(5000),
    )
    private_reply = frames[0].to_json()
```

The caller supplies the original logical operation ID. Each explicit call makes
fresh physical request metadata; uncertainty preserves that original ID and
never resubmits. Results are immutable `HttpEnvelope` values, with independent
JSON export. Errors use the common typed SDK kinds, canonical problems and
read-only/not-submitted/refused/uncertain certainty. A completed HTTP exchange
does not imply a durable operation completed: reconcile through explicit status
reads under current server authority.

Each request owns a one-shot Python worker started with the current interpreter;
origin, credential and request travel through private stdin. The absolute caller
deadline includes startup, DNS, TLS, network and decoding. Cancellation or expiry
terminates and joins the original worker, including a worker blocked in DNS.
The client binds to one asyncio loop, permits16 active requests and retains
cleanup ownership until the worker joins. Use `async with` or explicit `close()`.
No Rust executable is needed for this HTTP transport. Its private worker module
must be installed alongside the package.

Request JSON stays at64KiB, each response envelope at1MiB, the whole response
at2MiB and a batch at128 envelopes, with depth32/100000 value-node limits and
interoperable JSON integers. Correlation, repeated resource/cursor scopes,
terminal ordering, unsigned HTTP profile, content type and response framing are
validated. Native offers, live subscriptions and decoded media are outside this
finite transport. It does not advertise participant/native capability parity.

[HTTP component evidence](../evidence/gate-6/python-sdk-http.json) records180 local
functions, including21 actual local HTTP and3 verified HTTPS functions,80
unchanged source guards and34 negative type constraints. Core and HTTP test
phases have separate60s/30s runner budgets; request deadlines are unchanged.
The HTTPS fixtures use an ephemeral proper CA/leaf chain. The first positive
TLS test exposed a missing leaf Authority Key Identifier; adding that extension
makes strict OpenSSL verification pass while normal certificate and hostname
checks remain enabled. The shared Rust hosting fixture now emits that extension;
its new runtime check is pending Linux CI. All109 Node component/type regressions
pass with the corrected local certificate fixture.

These are actual HTTP sockets with synthetic local responses and owned Python
workers. They do not qualify a named remote endpoint, original Rust server,
installed clean-venv/restart/current-authorization journey or the complete SDK.
The expanded Linux Python matrix remains pending for this new source. Application
HTTP coverage, bounded synchronous ownership and helper deployment remain open
in G6-063, which stays in progress.

The [exactf79 Linux HTTP component](../evidence/gate-6/python-sdk-http-linux-f79.json)
is mixed. Python3.13 passes180 functions and34 negative type constraints.
Python3.11/3.12/3.14 pass the156-function core and type checks but fail HTTP
sent-deadline/cancellation fixture assertions;3.12 also reports a blocked-resolver
fixture failure. All80 tracked contents and modes match the exact commit. No
runner phase timed out or required forced cleanup. The older artifact does not
identify the failing assertion line, so startup timing or another cause remains
unproven. The runner now publishes only repository source paths/line numbers
from failures, excluding assertion text, private paths and raw logs. Those failed
profiles remain unqualified; no request deadline or submission assertion changed.

`SynchronousSdk` is the synchronous entry point to these same async types.
Transfer a fresh `Client`, `AdministrativeClient` or `CanonicalHttpClient` to one
owner. Call `run()` from its constructing thread with a typed async callback;
the owner invokes it on one persistent private event loop. Typed handles and
operation references can be retained between calls to that same owner.
The loop progresses during `run()` and `close()`; background event/status work
resumes on the next call rather than running between synchronous calls.

```python
from connect_sdk import Client, RequestOptions, SynchronousSdk

options = RequestOptions(3000)
with SynchronousSdk(Client(config)) as sdk:
    capabilities = sdk.run(lambda client: client.capabilities.refresh(options))
    # Run an existing async control journey with the same typed SDK.
    result = sdk.run(lambda client: my_async_control_journey(client, options))
```

The callback receives the exact typed client chosen at construction. Participant
and administrative roles remain separate; HTTP control retains its explicit
origin and trust selection. There is no second command implementation, retry,
background owner thread or alternate executable. asyncio may use its normal
platform child watcher. SDK calls retain their own `RequestOptions` deadlines
and thread-safe cancellation signal. User callback code has no additional outer
timeout: each SDK call must provide its options and custom async work must be
bounded by the application. A mutation's original certainty/key passes through.

An active async loop, foreign thread, nested/reentrant call, duplicate transfer,
already-bound client or closed owner is refused before invoking a callback.
After transfer, use the original client and retained async handles only through
this owner. Explicit `close(options)` has one cleanup budget. It closes the SDK,
drains the original helper tasks, cancels outstanding callback tasks and closes
async generators. It closes the loop only after pending work and original
processes have joined. Incomplete cleanup returns `cleanup-incomplete`, retains
the loop and rejects new calls; call `close()` again to drive the original cleanup.
A forced process kill is reported even after its eventual join. Context exit uses
five seconds. Callbacks must await their own work and must not create executor
jobs; this owner does not adopt application-created executors or external threads.

The synchronous component exercises repeated typed operation/handle calls with
one original helper, distinct administrative profile, actual local HTTP sockets,
pre-cancelled and sent cancellation/deadline certainty, thread/loop/reentrancy
refusal, callback errors, graceful close, forced child cleanup and bounded
stubborn-task refusal followed by retained-loop recovery. This qualifies the
recorded local component, not the installed original-server journey or G6-063.


[Synchronous component evidence](../evidence/gate-6/python-sdk-sync.json)
records193 local functions including13 synchronous ownership functions,
82 unchanged source guards,16 runtime modules plus the public outline passing
mypy and37 expected negative type constraints. Initial cleanup lost a callback
task that completed with SDK cancellation while closing the client; retaining
the initial task set before cleanup now retrieves that completion and closes
without unexpected async diagnostics. Runner/request deadlines are unchanged.
The installed original-server and expanded Linux scopes remain open.

[Installed HTTP/runtime evidence](../evidence/gate-6/python-sdk-installed-http.json)
qualifies a clean local wheel installation of this subset. Build tools are
hash-pinned in `sdk/python/build-requirements.txt`; the runtime has no third-party
dependencies. Two staged builds with the same source timestamp produce the same
wheel digest. All36 runtime files match source bytes, including the HTTP worker
and `py.typed` marker. The checker installs into a fresh venv, removes repository
import paths, then invokes an isolated `-I` consumer from an external directory.
Public async and synchronous HTTP control make six actual local synthetic
requests; all six installed-module workers use that venv's interpreter and join.
This proves the HTTP worker can deploy with the wheel. It does not qualify the
original Rust helper/server, restart, current authorization or the complete SDK.
The native `Client` and `AdministrativeClient` still require the deployed
`connect` executable on PATH, or the explicitly selected executable. Their complete
installed helper journey remains open.

To reproduce, prepare dependencies, create a private build venv, install the
hash-pinned build requirements there, then run:

```sh
python3 sdk/tests/check_python_installation.py \
  --output .local/gate-6/python-installed-check \
  --build-python .local/python-build/bin/python
```

The invoking Python needs pip; the clean consumer venv has no repository checkout
or inherited site packages. Builds, install and consumer phases have explicit
30s/30s/40s bounds with process-group cleanup on failure. Private logs, staged
sources and the venv stay under `.local/`; wheels stay under `dist/sdk/python/`.
The Linux Python matrix publishes installed-component evidence separately from
core/HTTP test evidence, including when another component fails. Its new source
still needs execution. No package publication occurs.

The [exact70 Linux evidence](../evidence/gate-6/python-sdk-linux-70.json) now
qualifies169 core functions, including13 synchronous ownership functions, and37
negative type constraints on all four Python versions. All four clean installed
HTTP checks also pass, each with six requests/six joined workers and36 runtime
files verified against source. The wheel digest is identical across the matrix.
This advances the recorded core/synchronous and installed HTTP subsets; it does
not qualify the native Rust helper/server or restart/current-authorization journey.

The same run retains HTTP test failures. Its failing source lines combine
request-arrival/worker-join or resolver-marker/no-effect assertions; the400/500ms
fixture callers can expire before the intended stalled phase. The
[lifetime fixture correction](../evidence/gate-6/python-http-lifetime-fixture.json)
uses the fixture's normal two-second caller deadline, retains every phase-entry,
original-ID, no-effect and joined-PID assertion, and adds an observed elapsed
bound below three seconds. The early unsent20ms test remains unchanged. Exhaustive
discovery now partitions core169/60s, HTTP protocol22/30s and deliberate lifetime
stalls2/10s. All193 local functions and37 type constraints pass; the changed
fixtures still require their own Linux evidence. These changes do not alter
product request limits, cancellation, retry or ownership policy.

The [exacta50 Linux matrix](../evidence/gate-6/python-sdk-linux-a50.json)
passes all193 component functions and37 negative type constraints on all four
Python versions. The separate installed HTTP subset passes everywhere, with six
requests/six joined workers,36 verified runtime files and identical wheel bytes
across versions. Every artifact matches its84 or86 tracked Git file contents and
modes. Core, protocol HTTP and deliberately stalled lifetime lanes all finish
without timeout, forced cleanup or unexpected async diagnostics. This resolves
the earlier Linux component failures for this source. The original native Rust
helper/server, restart/current-authorization and complete installed control
journey remain unqualified; G6-063 is still in progress.

The [installed native control checks](../evidence/gate-6/python-installed-native-local.json)
now pass EdDSA and ES256 locally on macOS arm64/Python 3.14.7. The clean wheel
consumer uses the default `connect` executable on PATH from an isolated external
venv with no repository import path. It checks typed capability/conflict errors,
exact-ID send/replay, signed finite journal/cursor exclusion, saved-reference and
same-cause recovery after an actual production-server restart, and fresh current
revocation refusals. Independent full Store and exact journal reads prove one
original message/operation and zero forbidden mutations. Each algorithm joins
five original helpers, five schema validators, the Python consumer and both server
instances. All 8,237 guarded inputs and the test executable stay unchanged, with
request 4s/phase 15s/function 90s limits. The initial protected-config overwrite
fixture failure is retained; the fixture now stages a 0600 file and atomically
replaces its owned configuration after restart.

`prepare_python_native_consumer.py` reuses the pinned wheel/clean-install checker
and verifies every installed runtime file before copying the public consumer.
The Linux hosting runner now has an explicit `--python-sdk` lane alongside Node;
its native Python execution is pending. This closes the recorded local message,
restart and current-authority subset. The complete installed control journey,
including sessions, movements, administration and decoded media, remains
unqualified; G6-063 stays in progress.

The expanded [installed control fixture](../evidence/gate-6/python-installed-controls-local.json)
passes both signing algorithms locally with the same clean Python 3.14.7 wheel.
It adds stable participant IDs and advancing membership epochs, stale-revision
refusals, incoming invitation source attribution, Session role/media/codec updates
and accept/reject/cancel/end decisions, native Connection quality/mute/hold/resume/codec
controls and Stream subscribe/unsubscribe. Typed message events agree with the
signed finite journal and independent Store causal fact across restart. The host
checks twelve completed control records independently and joins six helpers, five
validators and two server instances per algorithm. Each SDK request remains 4s,
each named phase 15s, and each whole function 90s. This is control evidence; handles
remain observations without Python media custody. Administration, durable operation
decisions and receipt coverage are still being completed for G6-063; package and
full parity qualification remain G6-069/G6-070.

The [installed administrative scenes](../evidence/gate-6/python-installed-administration-local.json)
now cover all eleven provisioning families, exact original receipt recovery after
restart, ordinary/revoked authority refusals and a real held committed response.
Cancelling the original wait preserves uncertainty; cancelling its queued command
leaves zero effect, and a fresh owner recovers the original without replay.
The six successful scene observations join sixteen helpers, eighty validators
and eight server instances in total. The two initial EdDSA command scenes refused
local validation before submission; a targeted unchanged-source run passed. Their
cause remains unresolved and the full six-scene run is recorded as failed.
The shared launcher also passes all six installed Node administrative regressions.
