# 013 — Gate 6 conversation model and lifecycle

Accepted implementation direction: 2026-10-15. Owner approved both Gate 6 plans.
G6-004–G6-009 specify this decision; runtime evidence is separate.

## Six nouns and their bindings

Conversation owns durable messages, Participants and Sessions. Identity is a
durable tenant-scoped entity. Participant is its conversation-scoped appearance;
kind is human/AI/system/external and role describes application responsibility.
Conversation membership, Session presence, role and effective authorization are
different dimensions. Device identifies a physical/software endpoint; an Endpoint
is a provisioned reachable binding with capability, verification and freshness.

Participant role is the Conversation default. A Session appearance snapshots its
role at admission and can carry an explicit Session-local override. Changing the
Conversation default does not silently rewrite other active Sessions. Handoff
normally transfers Conversation responsibility and the selected Session role;
an explicit Session-only mode is available. Neither role scope grants permission.

A Session contains live appearances of permitted Participants and Connections.
One Participant can use multiple Devices/Connections and concurrent Sessions.
A Connection carries negotiated Streams. Logical stream relationships can survive
replacement through explicit lineage; physical RTP and datagram bindings change.
IDs are stable bounded opaque identifiers; they are not source-of-authority fields.

## Lifecycles

- Conversation: explicit persistent creation is Connect's default; idle activity
  continues/resumes it. Only explicit authorized close ends it. An ephemeral
  policy may close after all Sessions end and the declared idle timer expires.
- Session: invitation is pending until acceptance and required connection/media
  readiness. A new Session starts once its admitted live participants are ready.
  Existing Session invitations add participants under explicit admission policy.
- Reconnect: losing the last Connection starts a default 30-second reconnect
  grace while no explicit end occurred. End after grace and no in-flight permitted
  negotiation. Explicit Session end terminates immediately and cancels grace.
- Membership leave/removal: revokes conversation access and increments membership
  epoch on re-admission. Session leave preserves conversation history access.
- Handoff: changes participant responsibility/media audience; Session and retained
  remote Connection remain. Device replace keeps Participant and changes its leg.
- Server restart: persists intent/history but does not resurrect network media.
  Interrupted or uncertain effects require truthful reconciliation.

The baseline's immediate last-connection end rule is replaced by the explicit
grace rule above. `session.update` topology changes use expected revision and
fenced transitions, never last-writer-wins bridge mutation.

## Ownership

Shared schemas, transitions and media primitives belong in UCTP/rvoip. Connect
owns durable application state, authority and orchestration. The app owns task
goals, prompts, scheduling, UI and business approval decisions. No new primary
Task, Approval, Handoff or Consultation noun is introduced.

See [canonical mapping](../../contracts/connect-uctp.md),
[audiences](../../contracts/connect-audiences.md),
[transitions](../../contracts/connect-transitions.md),
[operations](../../contracts/connect-operations.md),
[events](../../contracts/connect-events.md),
[media](../../contracts/connect-media-v2.md) and
[records](../../contracts/connect-records.md).
