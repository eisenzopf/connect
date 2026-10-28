# Typed Node consultation control

G6-062 remains in progress. `session.consult(input, options)` submits the public
Rust SDK consultation operation. It takes the controller Connection (`via`), the
original retained Connection (`retain`), nonempty private participants (`with`),
`silence` or `configured-media` hold treatment and an explicit topology revision.
The Node adapter captures these inputs before yielding. The helper owns no
alternate transition, native-binding or authorization state machine.

Completion returns a Session and immutable retained/admitted/retired topology
handles with its captured topology revision. An admission receipt or a metadata
handle does not establish that audio switched. Journal completion and actual
media evidence remain separate.

A consultation operation supports `returnToRetained` with a new, explicit
operation ID and topology revision. The helper restores the original public Rust
SDK operation, which uses its captured host-issued consultation record or performs
a current-authorized journal read if that record is not yet known. The logical
operation ID is never substituted for the host record ID. Return authorization
and the current topology assertion are checked by the host on the new mutation.

```ts
const consulting = await session.consult({
  via: controllerConnectionId,
  retain: originalConnectionId,
  with: { first: privateParticipantId, rest: [] },
  hold: 'silence',
  expectedTopologyRevision: current.topologyRevision,
}, { timeoutMs: 3000, operationId: savedConsultOperationId });
const saved = consulting.referenceJson();
await consulting.waitUntilReady({ timeoutMs: 3000 });

// Restoration performs no I/O and grants no media or return authority.
const original = client.operations.restore(saved, client.operationKinds.session_consult);
const returning = await original.returnToRetained({
  expectedTopologyRevision: later.topologyRevision,
}, { timeoutMs: 3000, operationId: savedReturnOperationId });
await returning.wait({ timeoutMs: 3000 });
```

Saved references preserve the full Conversation/Session/controller-Connection
scope and the retained Connection continuation. Consultation and return
continuation kinds are distinct and reject unknown fields. The captured original
host record also initializes the operation's cross-observation record guard.
Return references retain their own mutation identity and the original
consultation record. A return cannot reuse the consultation's logical ID.

TypeScript restricts `returnToRetained` to a consultation result operation through
its `this` type; the runtime independently rejects other operation kinds. Separate typed handoff and device movement controls are described in
[node-movement-sdk.md](node-movement-sdk.md). The current 59 local helper, HTTP
and negative declaration checks include captured consultation input, restoration
and fail-closed foreign topology/record cases.

An installed Node consumer now drives the existing actual native consultation
fixture through the public API. It covers private and restored duplex audio,
missing return grants, current revocation, original-record restoration, fresh
observation without media custody and remote EOF. The independent host journal
checks the return topology and whether a remote call continues; the Node return
result remains a scoped Session handle. Both signing algorithms, original helper
joins, server shutdowns and zero physical peers/media readers are required.

The new consumer passed a synthetic process check using an installed package;
all three original helpers joined. This checks the consumer protocol only.
Actual Ubuntu compilation/audio verification is pending. Each SDK request keeps
its four-second budget, each phase has an absolute 15-second cap, and each of the
two-algorithm functions has a 90-second cap. The previous native refresh failure
remains recorded. See the [native fixture evidence](../evidence/gate-6/node-consultation-audio.json)
and [typed consultation evidence](../evidence/gate-6/node-consultation-sdk.json).
