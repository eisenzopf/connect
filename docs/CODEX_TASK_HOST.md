# Codex task host installation contract

Status: selected Gate 4 host; the local grant/skill export component is qualified.
The packaged launcher passes local process fixtures; actual host/model qualification remains
G4-021/G4-023. The version observed locally is `codex-cli 0.162.0-alpha.17.2`.
The existing MCP endpoint enforces current task grants. Export/configuration
checks do not establish actual Codex skill loading or model recovery behavior.

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

## Packaged explicit continuation

After creating and issuing the protected task grant through the documented
[task-access workflow](TASK_ACCESS.md), export it with `export-codex`.
The export has a private project directory, reviewed Connect skill, MCP override
and completion manifest. It cannot mint or refresh authority. A registered
HostSession handle identifies this installation; the returned Codex session UUID
is separate adapter correlation, never a new grant or conversation ID.

The packaged launcher starts from a private UTF-8 prompt file:

```sh
connect task-access run-codex --installation .local/codex-review \
  --prompt-file .local/review-prompt.txt
```

Explicit continuation needs no retyped context:

```sh
connect task-access run-codex --installation .local/codex-review
```

It passes the same existing bearer through an environment variable, runs from
the exported directory and ignores global Codex configuration. Model-account
location/authentication is inherited, never copied or rewritten. The MCP override
requires Connect initialization and retains the ten-second tool/startup limits.
The Connect MCP entry sets `default_tools_approval_mode="approve"`, so configured
task tools need no per-action host prompt. This applies to that MCP server only;
current Connect task/grant authority still governs every operation. The installed
CLI accepts this mode and rejects an invalid mode during strict configuration
validation. See the [official MCP settings](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).
The [local component audit](evidence/gate-4/codex-delegated-autonomy.json) records
seven export/launcher/MCP behavior checks, strict lint and Rustdoc, and the actual
CLI configuration probes. This does not qualify an actual model workflow.

An intact earlier export without this policy field remains loadable. The launcher
uses the explicit policy for its child invocation without rewriting the saved
configuration, manifest, task mapping or grant lifetime. Other configuration
changes still require repair rather than silent adoption.

The launcher stores the installation digest, task/grant IDs, original prompt and
run ID before spawning. It stores the returned exact session UUID immediately.
An interrupted attempt retains its original prompt/run ID; a confirmed host turn
allows a new continuation intent on that same session. A host turn completing
does not mean the Connect task completed. Two processes cannot run the same
installation concurrently.

`--timeout-seconds` bounds the host turn to 1–300 seconds (120 by default),
independently of each MCP request budget. JSONL events are retained privately,
bounded to 4 MiB per attempt and 512 KiB per event; the installed Connect grant
secret is redacted. This adapter records host observations after emission. It
cannot by itself prove that a model persisted a communication mutation ID before
making a tool call; actual same-intent model recovery remains required in the
host/journey checks. The skill and Connect effect ledger supply the workflow
contract, not a substitute for that proof.

If the child exits before its session ID is saved, a later run refuses to start a
replacement. Inspect the retained intent and actual host session records through
protected operator repair. A changed task mapping, modified skill/configuration,
missing completion manifest, expired grant or active installation lock is an
explicit error. Current server revocation still denies MCP even if the saved
local grant file has not changed. Automatic wake, grant renewal and session
reconstruction are not implemented by this command.

The [local process proof](evidence/gate-4/codex-host-process.json) records four
behavior tests, strict Clippy/Rustdoc/format checks and retained failures.
Local process doubles exercise the packaged adapter against real loopback MCP.
They do not qualify Codex model support; G4-023/G4-034 still require actual host
behavior at the recorded installed version.
