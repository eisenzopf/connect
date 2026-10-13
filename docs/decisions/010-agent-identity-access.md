# 010 · One communication identity for an agent

Status: owner-authorized October 12, 2026. The frozen dc73996 implementation is
qualified in its recorded scope; owner-review repairs are qualified at 709cf824295c12963b5d62214877b91fcd750e8d. Amends the default local agent installation in decision 009. Retains
the existing task, invocation-grant and verified provider-binding contracts.

## Purpose

An installed assistant should be able to communicate as an identity across its
authorized conversations. It should not require a new task grant or trusted
renewal every few minutes for ordinary communication. The owner connects the
assistant once, can inspect its activity, and can revoke its credential.

The default local MCP installation uses a normal non-administrator communication
credential and the existing service operations. There are no per-action approval
prompts, permission registry or permission-management console in this gate.
The SDK, HTTP API and JSON CLI retain the same identity model.

## Identity, conversations and tracking

The key identifies the assistant. Conversation IDs select conversations that
identity may access; they do not confer access. Membership, current credential
scope, expiry, revocation and channel eligibility are checked by the existing
store/service path, including mutation replay and queued provider dispatch.
Model-supplied identity IDs, provider headers and retrieved instructions cannot
establish authority. Provider keys and operator credentials are not installed
in the model's tool profile.

Default setup grants the named supported communication operations, optionally
including voice and task management, with the existing broad recipient scope.
It does not grant wildcard administrative operations. Operators may configure
a smaller scope or shorter lifetime. The normal credential lifetime is the
existing configurable 30-day default; expiry is never silently renewed.

Tasks are optional records for work that needs reply correlation, checkpoints,
effects, waiting or cancellation. Creating a tracked task can derive its allowed
communication actions and concrete participants from current identity access.
The ordinary communication key is not bound to that task. Cancelling or expiring
one task stops that work without removing the assistant's other conversations.
Stable request identities still distinguish retries from new intent, including
task creation with derived defaults.

The conversation remains the home for context, files and session outcomes.
Tasks in the same conversation share authorized conversation context; tasks do
not create private sub-conversations. Voice sessions and task expiry do not end
the conversation. A new conversation requires explicit intent.

## Optional scoped delegation and provider binding

The existing narrow task-grant installation remains available when a developer
wants one invocation to operate within one task. Its five-minute ceiling,
protected renewal, cross-task isolation and current-authority checks remain
unchanged. It is an explicit mode, not the default local installation.

Vapi call/chat tools continue to use verified provider-resource bindings and
bounded invocation authority. Ordinary local keys must not reinterpret provider
headers as a trusted binding or bypass the provider authenticator. Existing
installations are not silently converted to another access mode.

## Observe and revoke

Connect retains attributed conversation events, request receipts and optional
task/effect records. Revocation stops future actions and replay under that key;
it does not erase the records or pretend to undo an already dispatched external
action. Unknown provider outcomes still require reconciliation.

An operator can revoke the assistant's credential and issue a separate current
inspection credential for the same identity. Existing authorized history and
task inspection remain available through that credential. Task inspection and
cancellation do not require the revoked backing credential to become valid
again; continuation of its work remains denied. An administrator of a different
identity does not gain a conversation-membership bypass.

SMS retains reviewed consent, approved program language, HELP/STOP/START and
last-mile suppression checks. A broadly usable agent key cannot override these
requirements. START cannot revive cancelled work.

## Qualification and delivery

G4-029–G4-035 qualify this correction within the existing 36-item plan. Earlier
item evidence remains proof of the delivered scoped mode, with its source and
limits preserved. It does not qualify the new default.

Required new proof includes one installed key across two authorized
conversations, independent incoming contact, retained context, optional tracked
work and restart/replay; current revocation, expiry, foreign access and queued
dispatch denials; retained owner inspection after revocation; and a fresh normal
package plus an actual-model journey using the delivered installation assets.
The existing narrow-grant and provider behavior must remain qualified.

The [dc73996 source/package and model evidence](../evidence/gate-4/final-source-composition.json)
records the completed pre-review qualification. The owner review then reopened
normal MCP reply consumption, retained same-owner inspection after original
credential revocation, hosted CI and public
reproduction/documentation work. The [numbered plan](../connect-gate-4-implementation.html)
records the completed repair cycle; earlier results keep their original source.

Gate 4 closes only after these checks and the remaining baseline checks pass.
This change does not add video, a full connector contribution kit, remote
deployment or community publication to Gate 4. Those remain in the roadmap.

Current owner-review qualification: Runtime 709cf82 passes 42 offline phases, 17 installed checks and 32 actual MCP calls with no errors. Qualification controls d64f1b9 combine the reviewed fixtures at 7bdd534 with all 3 hosted jobs. Local macOS arm64 owner review only; Gate 5, community publication and remote deployment remain separate.

[offline-709cf82](../evidence/gate-4/offline-709cf82.json), [package-709cf82](../evidence/gate-4/package-709cf82.json), [real-model-agent-709cf82](../evidence/gate-4/real-model-agent-709cf82.json), [review-remediation-709cf82](../evidence/gate-4/review-remediation-709cf82.json), [qualification-controls-d64f1b9](../evidence/gate-4/qualification-controls-d64f1b9.json)
