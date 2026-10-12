# Give an agent a Connect connection

Connect's default agent interface uses one regular, revocable communication
credential. Configure the identity once, then give its host one MCP URL and that
key. The agent can communicate across its authorized conversations, use the
configured tools and track work with task IDs. Routine communication requires
no per-task grant or five-minute renewal. The owner may still choose a narrow
task installation for a particular delegation.

The [installed dc73996 candidate](evidence/gate-4/package-dc73996.json) passes
all 17 local package checks. Its [actual-model proof](evidence/gate-4/real-model-agent-dc73996.json)
uses one regular key across two authorized conversations, writes source-backed
artifacts and questions, then resumes the exact host session after server
restart to handle a tracked reply and an independent incoming contact. The
immutable first/resume transcripts contain 30 successful tool calls and zero
failed calls. A separate public-SDK observer was corrected without new inference
or answer injection; its original failed reports remain failed. Full current
regression and whole-Gate 4 acceptance remain in progress in the
[numbered plan](connect-gate-4-implementation.html). Earlier scoped/provider
reports retain their own source and scope.

## Set up the identity once

Use an existing isolated local installation. Add its independent MCP listener
in the private server configuration, without a fixed invocation binding:

```toml
[mcp]
bind = "127.0.0.1:8193"
```

Create a named agent using the owner's local setup credential:

```sh
connect agent-access create-identity \
  --config .local/connect/operator.toml \
  --server-config .local/connect/server.toml --alias assistant
```

The result contains a non-secret identity ID and address. Repeating the same
owner/alias/request preserves the original identity receipt. A different intent
or an alias already created by another request produces a conflict. The optional
`--request-id` overrides the stable `identity-create-ALIAS` setup ID.

Issue the agent's separate communication credential with the existing CLI:

```sh
connect issue --config .local/connect/operator.toml \
  --server-config .local/connect/server.toml \
  --identity-id RETURNED_AGENT_ID --recipients '*' --voice --tasks \
  --output .local/connect/assistant.toml
```

The quoted `'*'` allows contacts without maintaining a per-contact list. It
expands recipients; it does not grant administrator operations. Conversation
membership, current credential revocation, channel capabilities and reviewed SMS
rules still apply. `--voice --tasks` adds the named media/work-tracking operations.
The credential defaults to 30 days; `--expires-days` accepts 1–365 days.

For the `review-loop` demo, initialization already creates `connect:agent@local`.
Resolve that existing identity instead of creating another alias, then issue a
new regular credential to a fresh output file. Existing demo files are preserved.
Owner setup credentials remain separate from the agent connection. Vapi and
Telnyx master keys stay with configured server workers.

## Install Codex tools and skill

Export the existing agent credential into a new private host directory:

```sh
connect agent-access export-codex \
  --config .local/connect/assistant.toml \
  --server-config .local/connect/server.toml \
  --mcp-address 127.0.0.1:8193 --output .local/codex-assistant
connect serve --config .local/connect/server.toml
```

Export validates current non-admin authority through a read-only database
connection. It writes the private `agent.toml`, supplied Connect skill,
`mcp-override.txt` and completion manifest. It issues no grant and changes no
existing credential or server state. Completed and interrupted output directories
are preserved; choose a fresh directory for another installation.

Authenticate the selected Codex CLI with your chosen account. Save a private
UTF-8 prompt file, then run one bounded turn:

```sh
connect agent-access run-codex --installation .local/codex-assistant \
  --prompt-file .local/private-request.txt
```

The supplied MCP configuration uses `bearer_token_env_var`, required startup and
bounded tool deadlines from the [official MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).
The launcher supplies `CONNECT_AGENT_CREDENTIAL` privately and approves the
configured Connect tools without per-action prompts. It inherits the chosen
model-account location, never unrelated provider/operator keys.

Later, explicitly resume the exact saved host session:

```sh
connect agent-access run-codex --installation .local/codex-assistant
```

The launcher persists original prompt/run intent before spawn, records the exact
session UUID and retains it across restart. An interrupted intent cannot be
replaced, and a missing session requires explicit repair. The shared bounded
process runner owns cleanup and redacts the agent key from captured events.
These are explicit invocations; the adapter does not supply automatic model wake.

An existing task-grant installation stays in its original access mode and uses
`task-access run-codex`. It cannot be converted or silently broadened while
recovering a saved task/session. Its optional grant/renewal mechanism still works.

## Use the same connection in other hosts

Configure the Streamable HTTP URL `http://127.0.0.1:8193/mcp` and the agent key
from its protected configuration using the host's bearer-secret mechanism.
The identity key is the same one used by the SDK or JSON CLI; no additional
registration or credential layer is required. A Codex configuration has this
shape, with the bearer stored in the environment rather than in the text:

```toml
[mcp_servers.connect_agent]
url = "http://127.0.0.1:8193/mcp"
bearer_token_env_var = "CONNECT_AGENT_CREDENTIAL"
required = true
default_tools_approval_mode = "approve"
startup_timeout_sec = 10
tool_timeout_sec = 10
```

Install the supplied [Connect skill](../skills/connect/SKILL.md) when that host
supports skills. Other hosts must preserve mutation inputs and report their
actual continuation mode; this configuration alone does not qualify them.

Ordinary tools select explicit conversation IDs and stable request IDs. Work
tracking is optional: create/inspect a task and use task-send/wait for a correlated
question. Authorized tasks can share conversation context while keeping their
questions and effects distinct. Ending voice preserves the conversation.
Normal sends, task sends and tracked replies have different schemas and reply
references; the skill explains those differences. A normal `connect_task_wait`
returns the native task wrapper: an attributed reply's received conversation
journal event ID is `data.data.events[*].data.data.event_id`. Use that ID in
`connect_task_send.reply_to` to authorize and correlate a follow-up. It does not
implicitly consume the reply's inbox marker. Explicit `consume_reply` is a
separate native task command, not an ordinary MCP tool. `connect_reply` instead
references the original tracked question's message ID when answering it.

## Revoke the agent connection

Use the non-secret credential ID printed by issuance/export:

```sh
connect revoke --config .local/connect/operator.toml \
  --server-config .local/connect/server.toml AGENT_CREDENTIAL_ID
```

Current revocation applies to MCP, SDK, CLI and replay. The key cannot be used to
recover by switching to the owner's credential. Preserve receipts and original
logical IDs when reconciling uncertain work. Revocation cannot undo a provider
operation that was already dispatched.

For expiry or rotation, explicitly issue a new regular credential under owner
authority. This adapter preserves the old installation and session intent rather
than replacing its saved credential automatically. Recovering or moving an active
session to different authority requires a separate explicit procedure.

## Inspect retained work after revocation

The owner's administrator credential does not bypass conversation membership.
To inspect work retained by the revoked agent, the owner can explicitly issue a
separate current credential for that **same agent identity**, keep it outside the
model and use the public task diagnostics:

```sh
connect issue --config .local/connect/operator.toml \
  --server-config .local/connect/server.toml \
  --identity-id ORIGINAL_AGENT_ID --recipients '*' --tasks \
  --output .local/connect/assistant-inspection.toml
connect task-diagnostics --config .local/connect/assistant-inspection.toml \
  --task-id SAVED_TASK_ID --codex-installation .local/codex-assistant
```

Current same-owner authority can read/cancel retained tasks and inspect effects
without reviving the revoked model credential or its old grants. The original
model connection remains revoked. Host diagnostics report saved session/run
state for the agent installation; one session may handle several tracked tasks,
so host-turn completion does not prove that a particular task or effect completed.
