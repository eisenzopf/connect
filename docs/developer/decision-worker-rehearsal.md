# Bounded G6-073 decision-worker rehearsal

The owner directly authorized the reviewed G6-073 action, and that one model
invocation is consumed. Its completed original proposal has been recovered
without another model turn. The original failed audit remains recorded.
See [actual-model and recovery evidence](../evidence/gate-6/decision-worker-model-authorized.json).
This document grants no further execution authorization.

The approved [product plan](../connect-gate-6-plan.html) states: “The plans do not
authorize agents, live calls, deployment or publication by themselves.”
[G6-093](../connect-gate-6-implementation.html#G6-093) requires concrete reviewed
resources and use of existing authorization where it covers those resources.

## Reviewed action

- One Codex CLI 0.164.0-alpha.11.1 proposal turn on the existing authenticated
  local account, with a host budget of 60 seconds and no automatic retry/resume.
  The pinned runtime chooses its default model; no model override is selected.
- Exact host executable:
  `/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`.
  SHA256 `a04752e4d46a4d89e53e6f61c59c5349a638c8ce172ffbf391fd482b4d902a4e`.
- The current normal-feature Connect source executable has SHA256
  `cc4f34787065d99b94b10104193b6a0faa83f8cd45a7ab9b7667a0401732e1b9`.
  It is the newly built source binary, not an installed daemon/package.
- Fresh ignored fixture state with directory mode 0700; two local synthetic signed identities,
  a fresh Conversation, request and exact attributed approval messages, one
  accepted proposal message, original status reads and membership-removal
  replay refusal. No PSTN, SMS, booking, public endpoint or communications
  provider is configured. No personal browser is opened.
- The model proposes `{action:"propose",text:...}` only. The request contains
  synthetic authorized event content, including an adversarial `approved:true`
  instruction, explicitly marked untrusted. The application validates output
  and independently obtains the exact signed owner decision before Connect IO.
- Controlled replay snapshots reuse this same retained answer to exercise
  invalid actions, speech in progress and stale Session metadata. They start
  no additional model turn and submit no effect. These metadata fault cases
  do not qualify physical speech/media or native Session transitions.

The parent native worker has a budget of 90 seconds; initialization has 30 seconds
and server shutdown 10 seconds. Model start refuses if fixture preparation takes
more than 5 seconds, leaving time for its 60 seconds of host execution and owned cleanup.
The launcher saves exact prompt/schema/argv, consumes the once-only authorization
before launch, and retains original JSONL, session ID and outcome. A timeout,
unexpected tool item, malformed response or cleanup failure remains a failure;
it cannot trigger another model execution or replacement mutation.

## Implementation and configuration

The callback uses [model-host.py](../../examples/parley-on-connect/worker/model-host.py)
inside [RecordedProposalModel](../../examples/parley-on-connect/worker/model.mjs).
Version, binary digest and account status are checked before its one turn.
Only account-location/environment basics pass to the host. Connect/admin/provider
credentials are not exported. User configuration, MCP servers, command tools,
apps, browser/computer tools, hooks, goals, agents and daemon startup are disabled;
the sandbox is read-only. A retained JSONL audit requires one completed original
turn and no tool/action items.

The CLI uses its documented [JSONL/schema interface](https://learn.chatgpt.com/docs/non-interactive-mode)
and [feature configuration](https://learn.chatgpt.com/docs/config-file/config-basic).
Current local version/help/feature metadata and the binary hash were checked
without starting a model. Parser/approval/custody tests are synthetic and do not
qualify a real model turn or runtime flag behavior.

[model-host.example.json](../../examples/parley-on-connect/worker/model-host.example.json)
is deliberately disabled. After explicit owner authorization, copy it to a fresh
file with mode 0600 under ignored `.local/`, record that direct authorization reference,
and enable `approved_model_execution`. Its sibling `.invocation.json` is created
exclusively and fsynced; reuse of that configuration refuses even if the original
request changes. Account credentials/files are neither copied nor inspected.

Then run the same native lane with `--model-config PRIVATE_APPROVED_CONFIG`.
Default native and npm runs omit this option and use the simulated model.
The actual lane is not part of default tests. Original credentials, logs, prompt,
request, worker journal and host session mapping remain private for recovery.

## Evidence and remaining scope

The source-bound native report records ordinary signed SDK events, exact current
approval, one original submission/status recovery, controlled policy replays,
membership refusal and owned cleanup. Earlier failed runs are retained.
See [native worker evidence](../evidence/gate-6/decision-worker-native.json).

The 76 component checks include real process exits after pending effect and
model-response fsync, output/resource/approval refusal, bounded process input,
unexpected host tool rejection and unapproved-host refusal before execution.
One real model turn completed within the bound. Its strict auditor initially
rejected two exact pre-turn startup diagnostics from the restricted host. The
corrected auditor accepts only those fixed messages in that position, once each;
all tool items and other errors still refuse. The read-only recovery binds the
original request/configuration/consumption/trace and checks no matching original
host is live. No model probe, retry or resume occurs. A new native worker process
then reused the retained journals, submitted one approved proposal, recovered its
lost reply and refused current membership withdrawal. The original host PID/exit
metadata was not saved before the failed audit and remains unknown; current
cleanup observations do not reconstruct it. Future launches save process outcome
before auditing.

G6-073 is complete in this local implementation scope. Linux, remote/provider/media
profiles, the complete Parley application and Gate6 acceptance retain their
separate approved tasks.
