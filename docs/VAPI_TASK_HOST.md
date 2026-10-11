# Installed Vapi task host

This operator entry point stages one existing Connect task for Vapi. Agents use
its configured SDK/MCP capabilities autonomously. Initial server configuration
and exact-resource recovery use the private operator credential; neither command
is an agent tool. This is Gate 4 composition, not the Gate 5 first-run wizard.

The [local CLI component checks](evidence/gate-4/vapi-task-host.json) pass,
including restart, uncertainty, expiry, shutdown and native voice/MCP behavior.
No real Vapi account/model, public callback ingress or installed release candidate
is qualified by these development checks.

## Preparation

Create the conversation and task through the public SDK or JSON API. Keep its
owner credential reference, original task ID, permitted operations and expiry.
For voice, also create and accept the audio Session through the task client and
configure its saved-assistant voice route. Keep the same conversation across
voice and messaging; a new voice leg is a new Session, not a new conversation.

Save a private JSON manifest (mode `0600`) with these fields:

| Field | Value |
| --- | --- |
| `profile` | `connect-vapi-task-host/1` |
| `definition.credential_id` | Reference of the authenticated operator credential; never its bearer |
| `definition.request_id` | Stable grant-issuance identity for this invocation |
| `definition.task_id` | Existing task, owned by that credential's identity |
| `definition.assistant_id` | Saved Vapi assistant ID |
| `definition.origin` | `call` or `chat_session`; standalone `chat` is unsupported |
| `definition.operations` | Explicit subset of the task's configured operations |
| `definition.expires_at` | Original bounded Unix expiry, within task and backing credential lifetimes |
| `state_directory` | New private setup directory, relative to the manifest or absolute |
| `session_id` | Voice only: accepted Session created by that task |
| `callback_bind` | Chat only: stable loopback TCP address with a nonzero port |
| `callback_url` | Optional advertised URL, otherwise the local `/mcp` URL |

Set `VAPI_PRIVATE_KEY` in the operator environment. The default API origin is
`https://api.vapi.ai/`. `CONNECT_VAPI_API_BASE_URL` can explicitly select an HTTPS
origin or a local simulator at `http://127.0.0.1:PORT/` or `http://localhost:PORT/`.
The private SDK and rvoip use the same origin. Plain public HTTP, embedded
credentials, path prefixes and selectors are rejected. The pinned private SDK
currently excludes IPv6 HTTP simulator origins. Local qualification clears
inherited provider credentials and supplies a fixture key and loopback origin.

## Run

```sh
connect vapi-task serve \
  --config /private/connect/host-owner.toml \
  --server-config /private/connect/server.toml \
  --input /private/connect/vapi-task.json
```

Use the existing server configuration; no database is initialized or erased.
This runs the normal server, including configured provider workers. Optional
`--http-bind 127.0.0.1:PORT` enables its local HTTP/JSON facade. The server prints
its actual local callback address and task/resource IDs, never the setup secret,
operator bearer or provider key.

Voice mode stages the existing rvoip media attachment. The SDK's subsequent
native audio attachment creates the call; starting this command alone does not.
The media lifecycle owns callback capacity, expiry and teardown. It refuses to
replace a previous setup on the same Session, including after restart. Create a
new Session on the original task/conversation for a new leg.

Chat mode binds its callback listener before creating the Vapi session. A
30-second setup deadline returns an error with the original private intent
retained when acceptance cannot be determined. Restart with the identical
manifest and state directory. A known session is independently verified and
reused; an uncertain creation is not posted again. Changing the task, operations,
expiry, assistant or callback URL cannot overwrite retained preparation.

The callback's original expiry closes only that endpoint; the native server and
conversation remain available. Every request still checks current task authority.
SIGTERM drains the endpoint and server. Ending the process does not undo a
provider resource that may already have been created.

## Callback mapping

An advertised URL requires operator-managed routing to the printed local
listener. For chat, configure the mapping to its fixed `callback_bind` before
starting. For voice, configure it after setup and before opening native audio.
The proxy must rewrite `Host` to the backend address and preserve Authorization,
Origin when present, and provider correlation headers. Backend host/origin
checks remain enabled. The manifest accepts HTTPS or explicit loopback HTTP,
exactly `/mcp`, without credentials, query or fragment.

This command does not create a tunnel, public listener, DNS record or TLS
termination. Current offline tests use an owned loopback simulator and no
provider account. Actual cloud reachability and Vapi tool/model behavior remain
separate Gate 4 qualification.

## Recover an uncertain creation

Stop the task host. Retain its original manifest, private directory and provider
account. Obtain the exact resource ID from that account, then run:

```sh
connect vapi-task reconcile \
  --config /private/connect/host-owner.toml \
  --server-config /private/connect/server.toml \
  --input /private/connect/vapi-task.json \
  --resource EXACT_PROVIDER_RESOURCE_ID
```

Reconciliation verifies the saved assistant, active resource, original nonce and
current task authority through the private provider API. It emits no creation
POST and never refreshes the grant. A wrong resource or cancelled task fails.
After a chat-session reconciliation, restarting the same manifest reuses that
verified session. Voice reconciliation establishes the original tool binding;
it does not reconnect audio, replace the saved leg or resume an ended call.

See [Vapi bootstrap](VAPI_TASK_BOOTSTRAP.md) for the underlying API and
[Gate 4 implementation](connect-gate-4-implementation.html#G4-022) for progress.
