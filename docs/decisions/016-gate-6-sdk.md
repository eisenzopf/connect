# 016 — Public SDK and server HTTP design

2026-10-15; G6-012. The October 25 G6-059 refinement in
[SDK_COMPATIBILITY.md](../SDK_COMPATIBILITY.md) resolves the illustrative syntax
below: mutation awaiting returns an Operation; explicit bounded wait yields its
typed completion. Contract proofs are not yet runtime SDK implementations.

## Public objects

All SDKs expose Client, Conversation, Participant, Session, Connection, Stream
and Operation handles. Handles contain stable IDs and delegate to the same
canonical service; they do not cache authority. Every mutation permits a supplied
logical operation ID and returns an observable Operation. Convenience awaiting
is bounded and can be cancelled without inventing rollback of committed work.

```typescript
const client = await Connect.connect({ endpoint, credential });
const conversation = await client.conversations.create({ participants });
await conversation.send({ to: [organizer.id], text: "Can we talk?" });
const session = await conversation.sessions.invite({ to: organizer.id, media: ["audio"] });
const consultation = await session.consult({ with: [owner.id, assistant.id], retain: organizerConnection.id });
await consultation.waitUntilReady();
await session.handoff({ from: assistant.id, to: owner.id, retain: organizerConnection.id });
const movement = await ownerConnection.replaceDevice({ endpoint: ownerPhone.id });
await movement.wait();
for await (const event of conversation.events({ after: savedCursor })) { /* app decisions */ }
```

Rust uses typed request structs and async handles; Python uses equivalent snake
case methods and context-managed async clients. Node/Python helpers are managed
package assets, with bounded framed IPC, explicit formats and owned-process
cleanup. A caller-supplied executable remains an advanced override, not first run.
Browser capture/render is separate from Node/Python programmatic audio/video/data
I/O. HTTP, CLI and MCP provide control/status; media frames use media interfaces.

## Provisioning and context

A trusted backend has separate administrative clients for identities, endpoints,
credential enrollment and policy. Ordinary browsers/agents receive scoped access
and contact handles. No provider secret or arbitrary unverified destination enters
a normal participant call. Endpoint choice is explicit and capability-aware.

Workers consume typed attributed events and operation status. Tasks and grants
remain optional higher-level workflows, not a credential ceremony for every
communication. The app owns scheduling and business approval decisions.

## Packages

The selected package candidates are Rust `connect-client`, npm
`@eisenzopf/connect`, and PyPI `rudeless-connect` (Python import `connect_sdk`).
Read-only registry lookups on October 15 returned 404 for all three names.
This does not reserve a name or verify publisher ownership. Recheck availability,
publisher credentials and the release version in G6-059/G6-069 before packaging;
choose a project-owned available name if necessary. No publication is claimed.
Install tests use the exact candidate archive.

## HTTP reference

The supported server HTTP route/command design is specified in
[connect-uctp](../../contracts/connect-uctp.md). G6-065 supplies OpenAPI and
G6-085 supplies the standalone reference with tested curl examples. Cover auth,
errors, pagination/events, logical operation identity, status/cancel/reconciliation,
files, admin boundaries and limits. Do not invent a REST resource hierarchy merely
to duplicate canonical conversation semantics.
