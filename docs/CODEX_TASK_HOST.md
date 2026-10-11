# Codex task host installation contract

Status: selected Gate 4 host; the local grant/skill export component is qualified.
The packaged launcher passes local process fixtures; actual host/model qualification remains
G4-021/G4-023. The version observed locally is `codex-cli 0.162.0-alpha.17.2`.
The existing MCP endpoint enforces current task grants. Export/configuration
checks do not establish model recovery behavior. Actual runtime discovery is
qualified separately below. Full skill injection and saved-call replay now pass
a scripted runtime check; real-model follow-through remains open.

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

The required renewal path uses protected composition: the owner/application
issues a new bounded grant with its current backing credential, then supplies it
to the host adapter. An expired model token cannot refresh itself. Cancellation,
backing revocation, expired delegation or lost membership prevents issuance and
effects. The backing credential is never passed to the model process.

The local `renew-codex` component passes its [26-check host and admission
audit](evidence/gate-4/codex-grant-renewal.json), including strict quality checks.
These use an offline host process double and real Connect MCP; actual-model,
HostSession renewal and installed-candidate qualification remain open. After
issuing a replacement through [task-access](TASK_ACCESS.md), use:

```sh
connect task-access renew-codex --installation .local/codex-review \
  --grant-file .local/renewed-task-grant.json
connect task-access run-codex --installation .local/codex-review
```

Renewal requires the exact original task, invocation binding and operation
list. It holds the host lock and atomically saves a private credential pointer;
it leaves the original export, session UUID, prompt and interrupted run intent
unchanged. The original `host-state.json` grant ID remains installation identity;
actual MCP authorization and server effect records use the selected current
grant. An incomplete or mismatched renewal is an error, never permission to
fall back to the original credential. A successful file adoption reports
`renewed-not-invoked`; it does not attest current server permission or delivery.
Every MCP invocation still checks current authority. The owner can automate
issuance/adoption around explicit resume without per-tool approval prompts.

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
secret is redacted. This adapter records host observations after emission. The
separate actual-runtime interruption check below verifies saved call intent before
Connect dispatch and exact-session replay under healthy local storage. It does
not qualify the packaged launcher with a real model or prove that failed host
storage prevents tool execution. The skill and Connect effect ledger supply the
workflow contract; host/journey qualification still requires useful follow-through.

If the child exits before its session ID is saved, a later run refuses to start a
replacement. Inspect the retained intent and actual host session records through
protected operator repair. A changed task mapping, modified skill/configuration,
missing completion manifest, expired grant or active installation lock is an
explicit error. Current server revocation still denies MCP even if the saved
local grant file has not changed. Automatic wake, automatic grant issuance and
session reconstruction are not implemented by `run-codex`. Protected adoption
of an owner-issued replacement is the separate `renew-codex` operation above.

## Owned process cleanup

The launcher gives each Unix host turn its own process group. On macOS, it
observes normal child exit without reaping the leader, signals that inherited
group, then reaps the leader. Timeout and stream errors also signal before
reaping. Holding the leader waitable prevents the group ID from being reused
before cleanup. An inherited stdout pipe cannot defeat the turn deadline. The
absolute deadline is fixed before journal/setup/spawn; cleanup does not restart
the turn's budget.

Darwin can return `EPERM` when a group contains only zombies, as shown in
[Apple's group-signalling implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c).
The exception path uses a native group snapshot bounded to 64 KiB and one
second. It accepts that result only when the unreaped leader is present and
every member is a zombie. A live member, missing leader or failed/oversized
inspection keeps cleanup unsuccessful; genuine permission failures are not
treated as completion.

Cancelling the Rust caller future signals the group synchronously and schedules
direct-child reaping on the active runtime. Normal errors and deadlines await
that reaping before the command returns. Original task, session, prompt and run
intent remain available for explicit resume; cleanup creates no communication
or replacement task. This is lifecycle ownership, not a restriction on which
configured tools an agent can use.

This does not contain deliberately detached sessions or guarantee cleanup after
the launcher itself is killed without running destructors. Runtime teardown
still needs an external supervisor for reaping guarantees. Only local macOS
arm64 is qualified; no cross-platform or arbitrary process-tree claim is made.
The [local cleanup audit](evidence/gate-4/codex-process-cleanup.json) records
19 behavior checks: six cleanup cases, the native live/zombie snapshot case and
12 host/diagnostic/autonomy regressions, plus strict Clippy/Rustdoc/format. It
retains the interim deadline and Darwin failures. The component check is
`gate4_codex_cleanup`; whole host and installed-candidate qualification remain
G4-021/G4-023/G4-033.

The [local process proof](evidence/gate-4/codex-host-process.json) records four
behavior tests, strict Clippy/Rustdoc/format checks and retained failures.
Local process doubles exercise the packaged adapter against real loopback MCP.
They do not qualify Codex model support; G4-023/G4-034 still require actual host
behavior at the recorded installed version.

## Actual local runtime discovery

The [runtime discovery audit](evidence/gate-4/codex-runtime-discovery.json)
uses the installed CLI's documented [app-server interface](https://learn.chatgpt.com/docs/app-server)
with an isolated exported Native grant. It discovers the enabled project skill,
lists the configured MCP tools, reads the original task and retains an empty
pending page/cursor. A fresh runtime reaches the same task. Undelegated
cancellation and current revocation fail without changing Connect's Store.

Run this external-runtime check explicitly:

```sh
CONNECT_TEST_CODEX_BINARY="$(command -v codex)" \
  cargo test --locked -p connect-server --all-features \
  --test gate4_codex_runtime -- --ignored
```

The ordinary Cargo suite labels this check ignored because Codex is an external
installation requirement. The qualified command executes it; a missing selected
executable fails. The fixture creates empty private HOME/CODEX_HOME roots, passes
only the fixture grant, configures an unused loopback model endpoint and starts
no model turn. Its actual skill discovery and MCP calls qualify runtime wiring.
This discovery check alone does not prove full skill injection, exact-session
resume, pre-invocation intent persistence or useful agent follow-through.
The production launcher continues to use `exec` and exact `exec resume`.

## Actual local call intent and interruption

The [call-intent audit](evidence/gate-4/codex-call-intent.json) records three
checks using actual `exec` and exact `exec resume`, the installed code-mode tool
interface, and protected HostSession grants. A scripted loopback Responses peer
provides tool calls; it performs no inference. Empty private HOME/CODEX_HOME
roots keep account credentials and global configuration outside the fixture.

While authenticated MCP admission is paused, the test reads the original
communication program from canonical Codex session files and verifies that
Connect's Store is unchanged. The full installed skill is present in the runtime
request context. After acceptance, the public client sees one question and the
task waiting. Killing the host and resuming its exact UUID replays the original
program from saved session input without duplicating the accepted effect.
Interruption before dispatch and revocation between persistence and dispatch
also pass. A revoked required MCP server stops a fresh attempt before a turn.

Run this separate external-runtime check explicitly:

```sh
CONNECT_TEST_CODEX_BINARY="$(command -v codex)" \
  cargo test --locked -p connect-server --all-features \
  --test gate4_codex_call_intent -- --ignored
```

All three tests execute in the recorded opt-in run. Ordinary Cargo runs mark
them ignored; a missing explicitly selected executable fails. The fixture
supplies its scripted provider and controlled admission endpoint directly; it
does not qualify production `run-codex` with a real model. This proves runtime
wiring, skill injection and healthy-storage process-interruption behavior at
the recorded version. Real-model instruction following, useful reply-driven
continuation, full installation journeys, storage-write failures, power loss
and complete descendant-process cleanup remain separate checks. G4-018,
G4-021 and G4-023 stay open.

## Actual local reply continuation

The [reply-workflow audit](evidence/gate-4/codex-reply-workflow.json) runs two
actual installed Codex sessions concurrently against one Connect server. Each
has its own protected HostSession grant, exported skill/configuration and empty
private account roots. Scripted loopback Responses peers perform no inference.

Both sessions send a question and retain a journal cursor in saved session
results. A native public recipient answers one exact question while both host
processes are stopped. The owned Connect server restarts its original database
and listeners; the authoritative Store is unchanged. Exact-session resume reads
the retained cursor from actual saved runtime input, catches up the reply and
recalls its authorized source. The continuation program uses that received body
and source reference in one visible follow-up. The other task remains waiting,
and neither host receives the other conversation's content. Another exact
resume replays the saved follow-up program with no database mutation.

```sh
CONNECT_TEST_CODEX_BINARY="$(command -v codex)" \
  cargo test --locked -p connect-server --all-features \
  --test gate4_codex_reply_workflow -- --ignored
```

The recorded opt-in run executes this one behavior test with zero ignored;
strict Clippy and repository formatting also pass. Four fixture failures are
retained, including the event/source ID distinction and protected attempt-log
collision. No earlier evidence is replaced. This check qualifies actual runtime
continuation, reply attribution and shared-server isolation with deterministic
programs. It does not prove real-model reasoning, full artifact and independent
incoming-contact journeys, production packaged-launcher model use or a final
installed candidate. G4-023 is in progress, alongside G4-018/G4-021.

Inspect a retained installation without launching or resuming it:

```sh
connect task-diagnostics --config .local/native-task.toml --task-id TASK_ID \
  --codex-installation .local/codex-task
```

The caller supplies separate current Native/owner authority for task reads;
inspection never converts the installed HostSession grant or falls back to a
backing credential. It reports private retained session/turn metadata without
prompts, bearer secrets or logs. An interrupted turn preserves its original
session; a missing session requires repair. A completed turn does not complete
the task. See [task diagnostics](TASK_DIAGNOSTICS.md).

## Shared-conversation draft journey (qualification pending)

`gate4_codex_draft_journey` exercises two exported installations in the same
persistent conversation. Each task uploads a different draft through MCP and
shares the returned artifact handle. A native participant retrieves and checks
its size, digest and bytes both before and after a Connect restart. The participant
answers only one question. Both exact host sessions resume; only the explicitly
bound task should produce a source-backed follow-up. The unanswered task must
remain waiting. Replaying the saved follow-up must leave the Store unchanged.

```sh
CONNECT_TEST_CODEX_BINARY="$(command -v codex)" \
  cargo test --locked -p connect-server --all-features \
  --test gate4_codex_draft_journey -- --ignored --test-threads=1
```

This check is newly added and awaiting its runtime and strict-lint results.
It launches the actual installed CLI through `run-codex`, with scripted loopback
Responses and empty account directories. It does not test model reasoning.
The two tasks intentionally share authorized conversation history; task reply
binding determines which work continues. This preserves cross-task context
without guessing that any incoming message answers both requests. Independent
incoming-contact dispatch, the delivered walkthrough and full G4-025 acceptance
remain open.
