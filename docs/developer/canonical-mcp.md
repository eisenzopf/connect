# Canonical MCP control migration

G6-065 is in progress. The current MCP implementation adds finite canonical
conversation, participant, Message, Session, Connection, stream-selection and
operation tools to ordinary identity access. The public composition checkpoint passes the new official-client function under
both signing algorithms plus6 current HTTP and2 Task/provider functions.
All9 MCP library tests also pass, including the2 new schema/parser checks. Canonical configured-server qualification is recorded separately below; actual
agent-host export/transition and remote/model profiles remain open. Exact scope is in `docs/evidence/gate-6/canonical-mcp-control.json`.

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

Older ordinary communication tool paths for create/send/history, Session
lifecycle, participant add/update/remove, legacy dial/end and request status are
removed from ordinary discovery. Canonical counterparts use current schemas;
named application reads, files, receipt preferences and optional Task workflows
retain their independently scoped interfaces. Full Task composition, file/admin
facade parity, actual host transitions and required profile qualification remain
open in G6-065.

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
or qualify an actual model, installed agent export or remote ingress.
