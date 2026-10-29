# Typed Node Session control

G6-062 implementation is complete in its [recorded scope](../evidence/gate-6/node-implementation.json). Full installed package/platform parity and remaining runtime failures are tracked in G6-069/G6-070/G6-096/G6-098. Earlier evidence below retains its original scope.

`NodeClient` now provides typed Session invitations,
current refresh, accept, reject, cancel, updates and end through the public Rust
SDK helper. Connection/Stream controls are described in
[`node-connection-sdk.md`](node-connection-sdk.md). Consultation, handoff, owned
invitation delivery, events and trusted administration remain unfinished parts of
the full Node contract. Programmatic media belongs to G6-064.

`conversation.sessions.byId(id)` constructs a scoped metadata handle without
I/O. `conversation.sessions.invite()` creates a Session and permits an empty
target list for a solo Session. `session.inviteParticipants()` extends an existing
Session and requires nonempty targets and its current topology revision. The
existing Session ID, intent and capability offer stay explicit.

```ts
import { sdkId } from '@rudeless/connect';

// Persist both operation IDs and complete original intent before submission.
const inviting = await room.sessions.invite({
  sessionId: sdkId<'session'>(savedSessionId),
  to: [],
  medium: 'voice',
  intent: 'local-session',
  capabilities: { audio: ['g.711-mu'] },
}, { timeoutMs: 3000, operationId: sdkId<'operation'>(savedInviteId) });
const session = (await inviting.wait({ timeoutMs: 3000 })).value;
const current = await session.refresh({ timeoutMs: 3000 });
const ending = await session.end({
  reasonCode: 1000,
  reason: 'complete',
  expectedTopologyRevision: current.topologyRevision,
}, { timeoutMs: 3000, operationId: sdkId<'operation'>(savedEndId) });
const saved = ending.referenceJson();
await ending.wait({ timeoutMs: 3000 });

// Local reconstruction followed by an explicit current-authorized journal read.
const original = client.operations.restore(saved, client.operationKinds.session_end);
await original.status({ timeoutMs: 3000 });
```

Session snapshots distinguish `revision: SdkRevision<'session'>` from
`topologyRevision: SdkRevision<'topology'>`. Session mutations take the topology
revision. The declaration checks reject a state revision in that position; the
host independently refuses a deliberately substituted stale numeric value.
The SDK preserves the caller's assertion and never silently retries with a newer
revision or a replacement mutation ID.

`session.update()` supports participant leave, role change, medium upgrade and
codec renegotiation. Nonempty media/codec collections use `{first, rest}`.
Mutations resolve to observable operations; completion comes from the original
journal. Accept/update/end complete to Session handles. Reject/cancel complete
to `SdkSessionDecision` with a scoped Session, decision state and topology
revision. A handle or control receipt acquires no native media ownership.

The added operation kinds are `session_invite`, `session_accept`,
`session_update`, `session_end`, `session_reject` and `session_cancel`.
Restoration requires the exact command and Conversation/Session scope and
refuses an injected Connection scope. Snapshot and completion decoding bind the
same original IDs. The shared helper deadline, cancellation, uncertainty and
original-process close rules still apply.

The actual fixture uses separately enrolled regular caller and peer SDK
configurations. Operator authority stays in the Rust harness. Both clients use
exported package APIs; they explicitly end accepted Sessions, complete rejection
and cancellation decisions, restore the original end operation after helper
restart, and close all original helpers. No provider, model or telephone call
is involved. This exercises control metadata and journals; decoded media and
the full installed SDK journey retain their own qualification requirements.

Run the source-bound checker documented in
[`node-typed-sdk.md`](node-typed-sdk.md). Exact source/build/package and runtime
scope is recorded in [`node-session-sdk.json`](../evidence/gate-6/node-session-sdk.json).
The earlier four-second initial-create deadline failures remain retained until
their cause is established; later positives do not erase those failed events.

The final installed local function passes both signing algorithms in7.66seconds
with5642source inputs unchanged, followed by four exact RustSDK/stdio regression
passes. All44synthetic checks and negative declarations pass. One initial run
failed because a successful Cargo invocation had emitted a strip ENOSPC warning
and left a2784-byte, nonexecutable helper. That failure and original bytes remain
private evidence. The verified rebuild explicitly sets
`--config 'profile.release.package.connect-server.strip="none"'`, checks the
CLI executable header/mode/size and `sdk-stdio --help`, and binds its SHA before
running the fixture. This is a functional build profile, not a performance pass.
