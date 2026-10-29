# Python canonical SDK

`connect_sdk.Client` is the asynchronous participant SDK. It owns one lazily
started `connect sdk-stdio` process using `connect-sdk-helper/4`. The public Rust
SDK still owns authentication, current authorization, the journal and operation
execution. The Python layer validates typed observations and original intent.
The historical `NativeClient` application interface is not a fallback.

The current control slice implements Conversation create/refresh, participant
add/update/remove/refresh, message send/receipts and typed original-ID operation
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
also includes Session/Connection/Stream controls, movement/consultation,
incoming/durable events, raw/HTTP parity, administrative provisioning, helper
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
