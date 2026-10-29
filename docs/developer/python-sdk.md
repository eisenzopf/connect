# Python canonical SDK

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
also includes HTTP parity, administrative provisioning, helper
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
