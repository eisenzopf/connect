# Typed Node handoff and device movement

G6-062 remains in progress. `session.handoff(input, options)` and
`connection.move(input, options)` delegate to the original public Rust SDK
Client. The helper owns no alternate transition, role, journal or native-binding
state machine. New operation kinds are `session_handoff` and
`connection_replace` (wire command `connection.replace`).

Handoff captures a controller Connection, retained Connection, distinct source
and target participants, optional prepared candidate, outgoing leave/observe
choice and explicit topology revision. A Session role change takes no
Conversation revision; a Conversation role change requires its separate branded
revision. These assertions are never replaced with newer observations or a new
mutation ID.

An optional consultation supplies the original branded operation reference.
The adapter preserves its saved host record and retained context; the Rust SDK
performs any original-record lookup within the remaining request deadline and
checks current authority. A caller cannot inject `consultation_record_id` in
extensions while supplying that reference. Restoration is metadata construction,
not a return/handoff grant or native custody.

```ts
const handingOff = await session.handoff({
  via: controllerConnectionId,
  retain: retainedConnectionId,
  from: sourceParticipantId,
  to: targetParticipantId,
  outgoing: 'observe',
  role: { scope: 'session' },
  expectedTopologyRevision: current.topologyRevision,
  consultation: originalConsultation,
}, { timeoutMs: 3000, operationId: savedHandoffId });
const saved = handingOff.referenceJson();
await handingOff.wait({ timeoutMs: 3000 });

// A separate movement uses a fresh current topology assertion and saved ID.
const moving = await connection.move({
  retain: retainedConnectionId,
  target: { endpoint: targetEndpointId },
  expectedTopologyRevision: later.topologyRevision,
}, { timeoutMs: 3000, operationId: savedMoveId });
await moving.wait({ timeoutMs: 3000 });
```

Movement targets exactly one Endpoint or prepared candidate Connection. Those
IDs and topology/Conversation revisions have distinct declaration brands.
Saved contexts bind the complete Conversation/Session/controller scope,
retained Connection and exact target/role intent. Incoming helper references
must match the original submitted context. Typed results require bounded,
disjoint topology IDs, the retained original, the captured candidate/Endpoint,
correct outgoing retirement and the expected Conversation revision advance.

Optional `roleChange` and `binding` fields expose captured host observations.
They confer no current role authority or media ownership. Topology handles do
not acquire another physical peer's native hop handles or historical offer
receipt revision. The helper and Node decoder independently retain these bounds.

The 59 local synthetic helper/HTTP functions and negative declarations pass
with 198 guarded inputs unchanged. An offline installed package exactly matches
the current generated distribution. The source checker also rejects coercible
arrays in operation/decision states and participant kinds; the previously
installed SDK accepted all four malformed synthetic outputs, while the current
SDK fences the original helper as a protocol failure.

Recorded Linux evidence now includes all24 original movement scenes with
independent native media peers and the installed public Node controller. Exact
`ca6fb159` passes the full Node26 lane, including these24 scenes with48 original
helpers and24 servers joined; see `docs/evidence/gate-6/node-native-ca6.json`.
Exact `972ed986` passes the Node22 lane; its Node26 scope retains a minimum-audio
failure. Earlier audio/EOF causes remain unresolved by later success. At that checkpoint, local Rust linking was skipped because of ENOSPC; current
local linking and candidate runtime results are recorded below.

The [native movement fixture](node-movement-audio.md) exercises role scopes,
outgoing dispositions, endpoint movement, cancellation, revocation and recording
failure. Current `node-movements/3` additionally requires two candidate-target
cases. An original signed target creates a preparing Connection at its separate
Endpoint; Node selects that exact ID and must still obtain the original managed
replacement ticket, generation-two IO and strict media boundaries. The expanded
matrix has26 cases,52 helpers and26 servers. All24 original cases remain required;
the two new cases failed the preparation-row check on both Linux matrices at
exact94. An ordinary pending offer has no media readiness and cannot supply a
private replacement ticket. The host now retires its exact original exchange
before acquiring fresh managed preparation and preserving the public candidate
ID at commit. Both complete candidate cases pass locally on `bf726ad7`, including
actual decoded audio, the subsequent Endpoint move and current key-loss/recovery
checks. See [current local evidence](../evidence/gate-6/native-candidate-production-local.json)
and the [retained candidate investigation](../evidence/gate-6/native-candidate-adoption.json).
The complete current local/Linux matrix and G6-062 closure audit remain open.
