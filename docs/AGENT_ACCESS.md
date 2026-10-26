# Give an agent a canonical Connect connection

G6-065 is migrating the agent-facing facades. Ordinary MCP access uses one
current, revocable Identity credential and explicit authorized resources.
Task tracking and narrowly delegated Task installations remain optional.
Current qualification and limitations are in
[evidence/gate-6/canonical-agent-export.json](evidence/gate-6/canonical-agent-export.json).
Actual-model, live-transition/media, remote and installed package profiles retain
their separate qualification requirements.

## Start a protected local installation

For a new isolated installation, the canonical bootstrap creates operator,
reviews and agent configurations, scoped credentials and native Device keys:

```sh
connect init --directory .local/connect --example review-loop \
  --bind 127.0.0.1:4433 --http-bind 127.0.0.1:8194
```

Existing installations keep their state and identifiers. Add an explicit MCP
listener to the private canonical `server.toml`, then start it:

```toml
[mcp]
bind = "127.0.0.1:8193"
```

```sh
connect serve --config .local/connect/server.toml
```

The ready output reports `mcp_streamable_http=http://127.0.0.1:8193/mcp` without
secrets. Public MCP bind addresses and provider-origin fixed bindings are
refused. Ordinary Identity access and separately verified native/host Task grants
keep their own ceilings. See [developer/canonical-mcp.md](developer/canonical-mcp.md).

## Export an existing scoped credential

Use the bootstrap's canonical agent configuration as the credential source:

```sh
connect agent-access export-codex \
  --config .local/connect/agent.toml \
  --server-config .local/connect/server.toml \
  --mcp-address 127.0.0.1:8193 --output .local/codex-agent
```

Export reads the selected installation's current credential metadata and verifies
the source Identity assertion. It refuses administrator, revoked, mismatched and
Task-grant authority before creating output. Control-only credentials are valid;
export does not require or add messaging permission. It issues no new credential,
changes no scope, and preserves the original identity and credential IDs.

The private installation uses `connect-codex-agent-install/2` and a minimal
`connect-agent-credential/2` file. Its `agent.toml` contains the MCP credential and
derived Identity/credential IDs. Device signing keys, TLS trust files, native
endpoints and operator credentials are not copied. The source's native key
metadata is not used as MCP authentication or media custody. The generated skill
assets and MCP configuration are hashed in the completion manifest; existing
complete or interrupted destinations remain intact.

Version 1 exports require their original tooling for inspection; the current
loader refuses their format. It does not convert credentials, overwrite saved
intent or silently resume an old host under another installation. Keep original
state and reconcile its work before selecting a fresh installation.

The exported MCP `agent.toml` is not a native or application CLI configuration.
Canonical JSON CLI calls need the separately configured native source; named
application/Task calls need their explicit application configuration.

## Run or resume the selected host

The launcher uses the project's pinned supported host profile and verifies its
version and model-account availability. Record the bounded designated
host/account/budget and cleanup configuration for actual-model rehearsals in
G6-093 before invoking that lane. Local fixture checks do not qualify a new host
version or an actual model turn.

```sh
connect agent-access run-codex --installation .local/codex-agent \
  --prompt-file .local/private-request.txt
connect agent-access run-codex --installation .local/codex-agent
```

The first call persists the prompt/run intent before spawn; the second explicitly
resumes its saved session. The launcher supplies `CONNECT_AGENT_CREDENTIAL`
privately, retains the exact session, bounds and joins its child process, and
redacts the credential in captured events. It does not refresh authority, replace
an interrupted intent or install an automatic wake. Its generated configuration
is defined by the [host configuration generator](../crates/connect-server/src/cli/task_access/codex.rs).
Other hosts need their own supported secret and continuation mechanism.

## Use current tools and recover original work

Discover the installed schemas and use `connect_inspect` for current identity,
ceiling and named tools. Canonical communication uses
`connect_conversation_create`, `connect_message_send`, `connect_message_history`
and `connect_operation_get`. Keep each complete intention, `operation_id`,
Message ID, audience and exact Conversation/Session/Connection scope before IO.
A new physical request does not need a new logical mutation. Query original
status after uncertainty; a completed control record does not prove decoded
media, provider delivery, reading or human action.

`connect_conversation_events` returns finite pages with `payload.live:false`.
The initial read supplies a snapshot/cursor. Resume with `payload.after` from the
last handled cursor; notifications are hints. Session consultation, handoff and
device movement use their advertised canonical payloads and topology revisions.
No tool argument supplies an actor credential, signature or physical media peer.

Named application reads, files and optional Task workflows retain their distinct
schemas. Task request/step IDs, attributed journal reply events, original question
Message IDs and explicit reply-consumption revisions are different references.
Use the installed [Connect skill](../skills/connect/SKILL.md) for these workflows.
A regular installation cannot adopt a narrow grant during recovery or consume
another Task's reply merely because it shares a Conversation.

## Revoke or remove

Use the non-secret credential ID printed by export and the explicit trusted
application configuration for revocation:

```sh
connect revoke --config .local/connect/operator.application.toml \
  --operation-id saved-agent-revocation AGENT_CREDENTIAL_ID
```

Current revocation applies to MCP, SDK, CLI and replay. It does not undo an
already dispatched effect. Preserve original operation references. Inspection
uses a separate current credential with actual Conversation/Task authority;
administrator status alone does not grant private history. The model connection
cannot substitute that inspection authority after its own revocation.

To remove only a verified current export:

```sh
connect agent-access remove-codex --installation .local/codex-agent
```

Removal preserves unrelated files and saved diagnostics. It does not revoke the
server credential or delete the source configuration. Credential expiry/rotation
and continuation under different authority require explicit owner action.

## Additional setup and historical qualification

`agent-access create-identity` accepts canonical owner/server configurations and
reconciles the original owner/alias/request ID. It creates the Identity only;
credential issuance and any required native endpoint enrollment are separate
trusted setup. The older generic `issue`/retained-inspection recipes are not a
qualified canonical workflow; their facade migration remains in G6-065.

Earlier [Gate 4 review](evidence/gate-4/acceptance.html),
[installed candidate](evidence/gate-4/package-dc73996.json),
[actual-model baseline](evidence/gate-4/real-model-agent-dc73996.json) and
[owner-review repairs](evidence/gate-4/review-remediation-709cf82.json) retain their
recorded source and local scope. They do not establish Gate 6 actual-model,
remote/provider, current package or publication acceptance. The
[agent journey runbook](AGENT_JOURNEY_QUALIFICATION.md) retains its own lane and
configuration requirements.

## Inspect retained Task work

Use an explicitly issued current Task-owner inspection credential in an
`ApplicationClientConfig`; do not reuse the revoked agent or a protected
HostSession grant as an ordinary owner. The endpoint is the installation's
protected HTTP application listener.

```sh
connect task-diagnostics --config "$CONNECT_INSPECTION_APPLICATION_CONFIG" \
  --task-id "$CONNECT_TASK_ID" --limit 100
```

The view contains bounded lifecycle/journal/source metadata, independent paging
cursors and conservative local host observations. It does not publish message
bodies, resume a host, consume replies or retry an effect. Missing optional read
permissions are explicit unavailable sections. Invalid cursors and current
reader revocation fail the sample. A current owner's ability to inspect old work
does not revive the old agent's authority. See the
[application diagnostic contract](developer/application-sdk.md#retained-task-diagnostics).
The older generic credential `issue` recipe still requires its canonical issuance
and recovery migration; no issued credential is fabricated by this command.
