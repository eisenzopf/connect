# Codex task host installation contract

Status: selected Gate 4 host; implementation and actual host qualification remain
G4-021/G4-023. The version observed locally is `codex-cli 0.162.0-alpha.17.2`.
This guide defines the package installation contract, not a claim that its
Connect installer or MCP endpoint already exists.

## Required installation flow

1. Install the qualified Connect candidate and initialize isolated local state.
   Keep existing server state and provider credentials intact.
2. Authenticate Codex using the user's chosen account. Connect never copies model
   account authentication into a task, grant or provider configuration.
3. Through the public Connect client, create a task in an existing conversation
   with explicit operations, concrete recipients and expiry. The trusted host
   adapter registers the exact task/session binding and issues a five-minute
   subset grant through protected composition.
4. The Connect host installer generates an isolated MCP configuration, protected
   bearer environment and Connect skill assets. Use authenticated loopback HTTP;
   the documented Codex `bearer_token_env_var` keeps the secret outside tool
   arguments. This is a packaged command, not user-written bridging code.
5. Start one bounded Codex turn. Persist its exact returned session ID alongside
   the Connect task and conversation before acknowledging the host binding.
   Refuse a second task's session or an ambiguous installation.
6. If the task is waiting, stop the model turn. Resume the exact saved session
   after a durable reply arrives. `codex exec resume <SESSION_ID>` is the
   documented host primitive. Never use `--last` for task continuation.

The initial supported mode is explicit resume through the packaged adapter,
without retyping context. A worker may trigger a bounded turn on that same
session; automatic continuation is claimed only after its separate tests pass.
Connect notifications are hints; the task journal provides missed replies.

## Credential refresh and failure behavior

The protected adapter retains the backing credential reference and verifies its
current authority before issuing a new bounded grant. An expired model token
cannot refresh itself. Cancellation, backing revocation, expired delegation or
lost membership prevents refresh and effects. Secrets are not printed in logs,
Debug records, task receipts or the model's instructions.

An unavailable model account leaves the task waiting, with a visible host error.
A disconnected MCP request returns pending/uncertain according to durable effect
state; resume reconciles the same logical ID. It does not create a second send.
Missing session state is an explicit repair error, not a new unrelated session.

## Qualification before claiming support

G4-023 uses the actual installed CLI in fresh isolated state with installed skill
and configuration. It proves discovery, question, stop/disconnect, durable reply,
same-session resume, useful follow-through and two-task isolation. Record CLI,
MCP SDK/protocol, candidate/source versions and failures. G4-034 repeats the
declared journeys against the final installed candidate. Offline mocks cannot
close these host items.

Primary sources: [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
and [noninteractive execution and resume](https://learn.chatgpt.com/docs/non-interactive-mode).
