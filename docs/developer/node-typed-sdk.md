# Typed Node control component

G6-062 remains in progress. `NodeClient` currently implements typed Conversation
creation/refresh, typed participant add/update/remove/refresh, text messages, operation recovery and
durable cancel/reconcile decisions. Its supported operation kinds are
`conversation_create`, `message_send`, `operation_cancel` and
`operation_reconcile`, plus the three `conversation_participant_*` kinds.
The [Session control component](node-session-sdk.md) also implements invitations,
accept/reject/cancel, updates, end and original-operation recovery. The full
Connection/Stream/topology, invitation delivery, event,
administration and remote HTTP interfaces remain required. The [raw canonical interface](node-raw-sdk.md) now delegates validation and finite requests to the original Rust SDK; actual qualification remains pending.
Programmatic media is G6-064. There is no fallback to the historical generic API.

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
