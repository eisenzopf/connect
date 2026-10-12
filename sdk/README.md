# Connect language SDKs

- Rust: `connect-client`, directly over verified UCTP/QUIC.
- Python: `sdk/python/connect_sdk`, Python 3.11+ on the qualified macOS host.
- TypeScript: `sdk/typescript`, Node 22+, build with `npm ci && npm run build`.
- HTTP/JSON: opt-in loopback `POST /v1/operations` on the Connect server.
- CLI: `connect request` for one operation; `connect stdio` for JSON-lines sessions.

Python and TypeScript `NativeClient` reuse the Rust SDK through a persistent
subprocess; they require a built `connect` executable and a protected client TOML.
Their `HttpClient` uses local HTTP/JSON instead. Transport modes are explicit.
Packages are source distributions for local review, not published npm/PyPI releases.

Every mutation needs a caller-owned `request_id`. Persist it and the exact inputs
before sending; reusing it with different inputs returns `conflict`. Provider
acceptance/delivery is separate from local acceptance. Inbox consumers save their
effects, processed event IDs and cursor atomically. The connector worker never
repeats an uncertain external mutation automatically.

See [the runnable presence example](../examples/agent-presence/README.md) and
`bash scripts/check-gate-2.sh` for local conformance tests.


The SDKs separate operations, protocol validation, errors, and transports.
TypeScript exports named result interfaces and a discriminated `Outcome` union.
Both consume the shared wire fixtures and reject malformed/oversized results.
Transport and protocol failures stop the native helper; close the client and
recover using the same logical mutation ID. Typed application rejections leave
the client usable. The control helper reports invalid configuration as
`configuration` and rejected credentials as `unauthorized`, including expiry on
an existing peer. After fixing configuration or explicitly renewing a grant,
start a new client; the helper never reloads credentials or borrows owner authority.

Run `npm test --prefix sdk/typescript` and
`PYTHONPATH=sdk/python python3 -m unittest discover -s sdk/tests -p 'test_*.py'`
for bounded helper/HTTP failure tests with no provider account. The complete local
runner includes these tests plus real Rust native-helper/HTTP parity and recovery.

## Voice lifetimes

Rust `NativeAudioSession` and Python/TypeScript `NativeAudioClient` carry raw
PCMU through native UCTP/QUIC. A reliable terminal event for the current media
binding ends reception promptly: Rust returns `ClientError::MediaEnded`, and
Python/TypeScript return `ConnectError` with code `media-ended`. Subsequent media
operations retain that code. Transport failures remain uncertain. End the
application-owned session explicitly when finished; a media-leg end never closes
its durable conversation.

The `@rudeless/connect/browser` entry exports `BrowserVoiceSession`. Its default
input is the microphone. Applications can supply a live audio-only stream with
`BrowserVoiceSession.open(url, credential, timeoutMs, { inputStream })`; the
session owns and stops the supplied tracks. Clone tracks to retain another owner.
The `ended` promise settles after either side ends the browser leg and capture
stops. Browser WebRTC uses loopback signaling; it is not a native browser QUIC SDK.

See [the conversation client](../examples/conversation-client/README.md) and
[explicit live rehearsals](../scripts/live/README.md) for hardware and generated
speech examples. Generated audio plus independent decoding can test the media
path autonomously; it does not qualify a particular microphone or speaker.

## Discovery and context inspection

Use `capabilities` with an address and optional conversation to inspect authorized
operations, configured routes, availability and limits. A configured route is not
a provider health check or a guarantee of delivery. The legacy `resolve` response
remains a static identity declaration.

Before opening browser voice, use `browser_binding` and verify that its conversation
and participant match the intended conversation. Its signaling URL belongs to that
binding; request microphone access only after this preflight succeeds.

`context_view` returns bounded recent receipts and separate omission counts.
Use `context_deliveries` to page older delivery receipts newest first. Cursors
are opaque and authorization is checked on every page, including after restart.
Python and Rust use snake_case helpers; TypeScript uses `browserBinding`,
`contextView` and `contextDeliveries`. See the shared wire fixtures for exact fields.

## Task control

Agents use the same regular, revocable identity credential as applications for
ordinary SDK, HTTP and CLI communication. Tasks optionally track work in their
authorized conversations. See [agent installation](../docs/AGENT_ACCESS.md).

The `connect-task/1` extension carries durable task control on the existing
native UCTP/QUIC connection or the explicit HTTP facade. Discover support with
`negotiate_tasks(address, conversation_id)` in Python/Rust or
`negotiateTasks(address, conversationId)` in TypeScript. An older server without
the advertised extension returns `unsupported-profile`; ordinary operations
remain usable. A new native connection needs discovery again.

For optional narrow delegation, a Native model grant can negotiate its exact
bound task without directory permission: call `negotiate_task(task_id)` /
`negotiateTask(taskId)`. Use the
protected SDK configuration exported by `connect task-access export-native`;
HTTP clients receive only that Native bearer. See [installation](../docs/TASK_ACCESS.md).
Communication under that grant goes through task Invoke, not primitive
operations or an owner fallback. Host/provider grants are not native credentials;
raw media and artifact HTTP permissions are not inherited. The server rechecks current task,
grant and parent authority on each command and replay.

Use `task(command)` with the typed command union. The result distinguishes task
records, inbox/reply/recall pages, effects, step claims and tool results.
`pending` means the task has more work; `uncertain` means the effect requires
reconciliation. Neither means completed. Keep task, conversation, step and
logical request IDs across reconnects. Save returned cursors with the effects
of consuming their events; cursors themselves never grant access.

An ordinary identity credential with the named task operations can use these
methods for optional owned work. Narrow delegated invocations use their
separately authenticated, task-bound adapter; these methods do not issue or
convert a credential into an invocation grant. Ownership, credential secrets
and verified invocation bindings are not task command arguments.

```python
from connect_sdk import NativeClient

# client.toml contains an explicitly scoped developer credential.
with NativeClient("client.toml", executable="connect") as client:
    client.negotiate_tasks("connect:human@local", conversation_id)
    status = client.task({"action": "get", "task_id": task_id})
    assert status["kind"] == "record"
```

```typescript
import { NativeClient } from '@rudeless/connect';

const client = new NativeClient('client.toml', 'connect');
try {
  await client.negotiateTasks('connect:human@local', conversationId);
  const status = await client.task({ action: 'get', task_id: taskId });
  if (status.kind === 'record') console.log(status.data.state);
} finally {
  await client.close();
}
```

Task inbox/wait requests receive their bounded server wait, at most ten seconds,
in addition to the exchange timeout. This does not extend mutation deadlines or
enable automatic retry. A transport failure closes the native helper; reopen,
discover and reconcile using the same IDs and exact inputs.

The JSON CLI discovers task support for each one-shot request:

```sh
connect request --config client.toml --task-address connect:human@local --envelope < operation.json
```

`operation.json` uses the `op: "task"` wrapper, with
`request: { task_profile: "connect-task/1", command: ... }`. `--envelope` emits
the control `Response` for success and application rejection; rejection exits
nonzero. Without it, existing successful requests still print an `Outcome`.
Inputs are bounded to 64 KiB and diagnostic errors do not echo request bodies.
The shared [task matrix](tests/task_matrix.json) exercises real-server behavior
through Rust, HTTP, CLI and both transports of the Python/TypeScript clients.
Its 32 steps include explicit assignment of an unassigned reply, recipient
assignment denial, attributed owner assignment and matching replay after
reconnect. Message IDs and recipient journal event IDs are distinct: assign
the event returned by the task reply inbox. The separate narrow-grant matrix
checks the Python/TypeScript native and HTTP clients, browser HTTP client and
JSON CLI without an owner credential fallback. HTTP remains an explicit
facade; the native clients use the UCTP/QUIC helper.
