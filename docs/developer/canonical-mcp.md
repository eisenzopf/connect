# Canonical MCP control migration

G6-065 is in progress. Ordinary identity access exposes finite canonical
conversation, participant, Message, Session, Connection, stream-selection and
operation tools. The official-client checkpoint and all9 MCP library tests pass;
exact scope is in `docs/evidence/gate-6/canonical-mcp-control.json`. Configured
hosting, scoped-agent export and native handoff have separate local evidence.
Actual supported model-host transitions, complete facade parity and required
remote/model profiles remain open.

Each tool advertises the canonical payload schema and explicit resource scope.
The schema comes from the checked UCTP contract with only reachable definitions.
The service validates the typed envelope again. Inputs contain no credential,
signature, transport peer or caller-selected actor. Mutations require the saved
original operation ID. Event reads require `live:false` and return finite cursor
pages; a tool call does not acquire a live event lease or native media custody.

An ordinary admitted IdentityAccess retains a private bearer binding. Canonical
execution checks that the handle belongs to this installation, then uses the
same current credential, protocol-floor, assurance, granular permission and
before/after response checks as canonical HTTP. This path creates no signed
physical peer. Optional task/grant access keeps its separate existing catalog
and cannot call the ordinary canonical catalog to broaden its ceiling.

`kind:canonical` results preserve the current connect/2 reply batch and status.
A canonical refusal is an MCP structured error with the original canonical
problem/envelope. Successful ACK, accepted operation state and completed media
metadata remain distinct from decoded audio, provider delivery and human action.
After uncertainty, use the original operation and resources with
`connect_operation_get`. No tool automatically prompts a human, retries a
mutation or changes transport.

Ordinary identity service dispatch uses the same named-operation partition as
application HTTP. A sealed identity handle cannot reopen a retired standalone
operation. `connect_message_receipt` reports `delivered` only; use
`connect_message_read` for a canonical read report with its saved operation ID.
Current receipt preferences still apply. A separately authorized Task invocation
retains its nested operation contract, original IDs and current scope checks.

Older ordinary communication tool paths for create/send/history, Session
lifecycle, participant add/update/remove, legacy dial/end and request status are
removed from ordinary discovery. Canonical counterparts use current schemas;
named application reads, files, receipt preferences and optional Task workflows
retain their independently scoped interfaces. Full Task composition, file/admin
facade parity, actual host transitions and required profile qualification remain
open in G6-065.

The ordinary dispatch/receipt checkpoint has before/after actual CLI probes,
all 27 retired selector refusals through the identity service, nested Task
exactly-once replay, both signing algorithms through the official MCP client,
and 280 current HTTP/schema exchanges. Its
[recorded evidence](../evidence/gate-6/canonical-mcp-application-dispatch.json)
keeps these scopes separate from remaining consultation, movement and host work.

## Canonical server configuration

MCP is opt-in and runs on its own protected loopback listener. Add this to the
canonical server TOML before `connect serve --config ...`:

```toml
[mcp]
bind = "127.0.0.1:4435"
```

The CLI reports `mcp_streamable_http=http://127.0.0.1:4435/mcp` after startup.
No bearer appears in the URL or ready output. Ordinary scoped credentials use
current identity admission; explicitly installed native/host grants retain their
separate ceilings. An optional fixed registered binding keeps that exact origin.
Provider bindings and public bind addresses are refused before startup, including
when the native listener uses the Remote profile. Port zero is useful for isolated
fixtures; an installed host needs the reported concrete endpoint.

The server owns the MCP listener, readiness, shutdown and drop. It drains MCP
before retiring the canonical installation. Failed bind releases startup
ownership so the same installation can start after its port is repaired.
Qualification is in `docs/evidence/gate-6/canonical-mcp-hosting.json`. This is local
control hosting; a running MCP listener does not establish native media custody
or qualify an actual model or remote ingress. Canonical agent export uses the scoped bearer described in [Agent access](../AGENT_ACCESS.md). Its minimal MCP credential file cannot replace the signed native client configuration required for canonical CLI control.

## Native handoff through MCP

The official RMCP client requests `connect_session_handoff` through the configured
listener while independent signed native peers own and decode actual PCMU media.
Both signing algorithms cover Session/Conversation role scopes and observe/leave
outgoing dispositions. Invalid consultation policy or missing consultation record
preserves failed original state, zero handoff effects and private media audience.

`connect_operation_get` recovers the same completed record and result from a fresh
MCP client. Explicit replay after outgoing retirement retains one effect/attempt.
The fixture independently checks the canonical HTTP record, marked bidirectional
audio, observer receive-only behavior, participant revisions and owned teardown.
The MCP result remains metadata; the independent native decoders supply audio
proof. Local evidence is `docs/evidence/gate-6/canonical-mcp-handoff.json`. This
direct protocol-client fixture does not qualify an actual model-host turn.
