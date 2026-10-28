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

Actual positive native Node handoff/move/consultation journeys remain pending.
The Ubuntu installed control fixture now additionally asserts current-authority
refusal for handoff and movement on ordinary configurations lacking those
grants, preserving original IDs, scope and helper usability. Its execution and
new Rust helper compilation are pending. Local Rust linking is skipped because
of the retained ENOSPC failure; no runtime pass is inferred. Existing recorded
native Connection control evidence remains a separate earlier source cohort.
See [the evidence record](../evidence/gate-6/node-movement-sdk.json).
