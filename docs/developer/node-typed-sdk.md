# Typed Node control component

G6-062 implementation is complete in its [recorded scope](../evidence/gate-6/node-implementation.json). Full installed package/platform parity and remaining runtime failures are tracked in G6-069/G6-070/G6-096/G6-098. Earlier evidence below retains its original scope.

`NodeClient` implements typed Conversation,
Participant, Session, Connection and Stream controls, text messages, original-ID
operation recovery, durable decisions and topology movement. Dedicated components
cover [native controls](node-connection-sdk.md), [movement](node-movement-sdk.md),
[incoming invitations](node-incoming-sdk.md), [durable events](node-events-sdk.md),
[trusted administration](node-administration-sdk.md), [raw canonical control](node-raw-sdk.md)
and [explicit HTTP/HTTPS control](node-http-sdk.md).

Current local source `bf726ad7` passes both signing algorithms for the installed
Node candidate movement and its subsequent Endpoint move, with actual decoded
native audio, current authority and original helper/server drains. The same
executables pass Node control profile 8, HTTPS profile 3 and the installed Python
native restart/authority regression. The
[current local evidence](../evidence/gate-6/native-candidate-production-local.json)
records exact source/executable/package hashes and scope. This is macOS arm64
with Node 26; the full current Node 22/26 Linux matrix remains part of installed parity
qualification in G6-070. The frozen API implementation audit is complete. Programmatic media remains G6-064.

The frozen public replacement method is `Connection.replaceDevice()`. The current
installed package passes both candidate/Endpoint replacement cases and native
control profile 8 through this API; all 109 component checks pass. A preceding
full local matrix passes 34 movement, administration and consultation functions
with all 78 helpers and 38 servers joined. See the exact source and scope in
[node API evidence](../evidence/gate-6/node-frozen-name-local.json). Linux failures
remain open; these isolated results were followed by the complete implementation audit linked above; full installed parity remains G6-070.

Earlier Linux source evidence in `docs/evidence/gate-6/node-native-972.json` qualifies
the recorded native control/invitation/raw/receipt and administration scenes on
Node22/26 with both signing algorithms. Node22's full recorded lane passes;
Node26 retains a minimum-audio failure, and earlier audio/EOF causes remain
unresolved. At that checkpoint, event resume and production HTTPS still awaited Linux
execution. Current evidence above and later linked artifacts supersede that
pending status; complete SDK, acceptance and publication remain open.
Programmatic media is G6-064. Transport selection is explicit.

Install the matching trusted `connect` executable. This component invokes
`connect sdk-stdio --config /absolute/path/to/sdk-client.toml`, with one fixed
`connect_client::sdk::ClientConfig` for the original process lifetime. That TOML
contains `tenant_id` and a `[control]` table with the native configuration. It is
distinct from the bare `CanonicalNativeClient` configuration. Relative native
trust and key paths resolve against this TOML. Protect the configuration and key
files; Node receives no operator or provider credentials.

The server crate owns only the bounded JSON-lines/process adapter. Typed helper
actions live in `connect-client::sdk::helper` and delegate to the public Rust SDK.
Rust owns native trust, signature verification, current authorization, result
validation and journal polling. Node performs framing, correlation, typed result
and stable record checks. It does not independently verify native signatures.

```ts
import { NodeClient, sdkId } from '@rudeless/connect';

const client = new NodeClient({
  config: '/absolute/path/to/sdk-client.toml',
  executable: '/absolute/path/to/connect',
});
const options = { timeoutMs: 3000 };
try {
  const room = client.conversations.byId(sdkId<'conversation'>(savedRoomId));
  // Durably save this ID and the complete original intent before submission.
  const operation = await room.send({
    messageId: sdkId<'message'>(savedMessageId),
    to: { first: sdkId<'participant'>(recipientId), rest: [] },
    text: savedText,
  }, { ...options, operationId: sdkId<'operation'>(savedOperationId) });
  const saved = operation.referenceJson(); // Persist before waiting.
  const completion = await operation.wait(options);
  console.log(completion.value.boundary); // local-mutation

  // Local reconstruction. Neither call submits, queries or restores a grant.
  const original = client.operations.restore(saved, client.operationKinds.message_send);
  const same = client.operations.recover(original.reference);
  await same.status(options); // Current-authorized read of the original journal.
} finally {
  await client.close(options);
}
```

Reference schema `connect-operation-reference/1` is shared with the Rust SDK.
References retain command, logical ID and coherent scope, are immutable, and
carry a result-type brand. Restore checks the schema, command, supported scope
and 4 KiB bound without I/O. Recover accepts an owned reference rather than a
structural object masquerading as one. Saved references are metadata, never
credentials. An owned operation rejects a changed host record ID across calls.

Submission returns an observable operation. `wait()` resolves only after the
Rust SDK reads a completed journal result; `waitUntilReady()` accepts ready,
committing or completed observations. Unknown and interrupted states are never
automatic resubmissions. Cancelling a local wait does not submit a durable
decision. `operation.cancel()` and `operation.reconcile()` require their own
mutation IDs and return separately recoverable decision operations. Reconciliation
supplies an evidence ID which the host independently verifies.

Every call captures intent before queuing and has an absolute deadline covering
local capture, queue, write and response. The remaining budget reaches the Rust
SDK. At most sixteen exchanges are owned, with one submitted at a time. Unsent
cancellation removes that queued request; submitted cancellation, expiry or
corrupt output fences and stops the original helper. There is no automatic
restart, credential reload or retry. Reopen explicitly and query the saved
reference before choosing recovery. Fixed loaded configuration survives file
edits. Idle close sends EOF and joins; incomplete cleanup is reported explicitly.

`NodeSdkError` preserves kind, certainty, original operation key, safe canonical
problem and any recorded observation. This includes trust, receipt-unavailable
and recorded distinctions from the public Rust SDK. Raw helper messages and
stderr never become application diagnostics. The older `ConnectError` belongs
to the historical generic interface; it is not an alternate error path here.

Run `python3 sdk/tests/check_canonical_helper.py --output .local/gate-6/FRESH`.
The 44 tests and negative declaration checks cover synthetic owned helper/HTTP
behavior. They make no cryptographic or full SDK qualification claim.

For the installed real-server component, pack locally into `dist/`, install
offline with scripts disabled in a fresh `.local/` project, and copy
`sdk/tests/node_sdk_server_consumer.mjs` there. After preparing dependencies,
build `gate6_canonical_runtime` with `test-support`, set
`CONNECT_GATE6_NODE_CONSUMER` to that copied script and `TMPDIR` to a fresh
private `.local/` directory. Run the exact ignored test
`canonical_stdio::node_sdk::installed_typed_node_sdk_recovers_typed_operations_and_current_authority`
with `--exact --ignored --nocapture --test-threads=1`.

The fixture creates synthetic regular caller credentials and owns both signing
profiles. It checks typed Conversation/message/decision completion, native/HTTP
record agreement, one message after explicit replay, fixed configuration,
original-ID recovery across helper restart, current revocation and four original
helper/two server joins. It starts no model, provider or telephone call. See
[`node-typed-sdk.json`](../evidence/gate-6/node-typed-sdk.json) for exact evidence,
retained failures and remaining scope. Local packing is not publication.

Participant handles retain their stable Participant and Conversation IDs.
`conversation.participants.add()` requires the current `expectedRevision`;
`participant.update()` and `participant.remove()` take
`expectedConversationRevision`. These are `SdkRevision<'conversation'>` values
returned by current Conversation refresh or explicitly validated by
`sdkRevision<'conversation'>()`. A topology revision is a different type.
The host checks the current revision; the SDK never rereads and silently retries
with a different assertion. Removing membership makes current refresh report
`receipt-unavailable`. Re-adding the identity retains its Participant ID and
advances its membership epoch. Restoring an older operation reference never
makes its historical membership receipt a grant into that new epoch.

The isolated Linux workflow now includes separate Node22/Node26 synthetic jobs
and generated-distribution checks. Their results require their own downloaded
artifacts; the local macOS passes do not qualify those CI profiles.

Current participant evidence and its limits are in
[`node-participant-sdk.json`](../evidence/gate-6/node-participant-sdk.json).
Two uninstrumented runs timed out on the first Conversation-create request at
its unchanged four-second budget, before membership assertions. A subsequent
instrumented run passes both signing algorithms and all membership/recovery
assertions; the initial deadline cause remains unproven. This is an explicit
open finding, not a completed uninstrumented profile or production fix.

A later uninstrumented run on the same executable and current installed package
passes both algorithms at the original request budgets. The earlier deadline
cause is still unproven. The downloaded Ubuntu Node22/26 artifacts independently
pass all41synthetic checks, negative declarations and generated dist equality;
see [`node-linux-control.json`](../evidence/gate-6/node-linux-control.json).
Neither result qualifies the remaining full SDK or resolves the failed events.

The isolated Ubuntu x86_64 native job now has independent Node22/26 cells, each
running the same source-bound fixtures and preserving all request/phase/function
bounds. Native evidence artifact names include the Node major version. Configuring
this matrix does not qualify either runtime; each cell must provide actual results.
See [the runtime matrix record](../evidence/gate-6/node-native-runtime-matrix.json).

The [incoming invitation component](node-incoming-sdk.md) now provides an owned
reader with cancellation/timeout retention, scoped Session decisions and local
close without stopping control. It requires the matching participant helper3;
the administrative helper2 remains separate. Current90 component checks and8
installed incoming checks pass; current Rust/native qualification is pending.
The historical [b2 native evidence](../evidence/gate-6/node-native-b2.json)
qualifies all11 administrative command families only in its recorded scopes,
while retaining the later revocation assertion and Node22 audio-boundary failures.

Incoming now follows the approved captured-options async iterator:
`client.incomingInvitations(options)`, `for await`, original inviter `from`,
and `close()`. The earlier per-wait draft is superseded. All 94 component/type
functions and 9 installed incoming functions pass; current real native iterator
qualification remains pending. See [the contract alignment](../evidence/gate-6/node-incoming-contract.json).
