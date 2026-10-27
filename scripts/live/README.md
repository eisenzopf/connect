# Explicit agent and live provider qualification

These checks are invoked separately from offline qualification. The normal-agent
journey uses an isolated local server and real model inference. Provider rehearsals
use a running, configured Connect server and real provider service. Both can incur
account usage; these drivers never purchase numbers or change provider accounts.
Offline tests exercise the native driver against explicitly injected loopback
provider fixtures with no account credentials; they do not qualify live service.

`captured` means audio was collected, **not** that a test passed. Decode the
received recording independently, compare the spoken answers to conversation
facts, and inspect the durable session/context outcomes. Keep credentials,
recordings, transcripts and full provider responses under ignored `.local/`.
Publish only redacted behavior evidence. Preserve every failed attempt.

## Gate 4: the actual normal-agent journey

The public [agent-journey.py](agent-journey.py) runner repeats the normal-key
draft/reply and independent-contact exchange using an installed Connect
candidate and an explicitly selected, authenticated Codex CLI. Follow the
[agent journey runbook](../../docs/AGENT_JOURNEY_QUALIFICATION.md) for prerequisites,
the complete observer assertions and failed-attempt handling:

```sh
python3 scripts/live/agent-journey.py /absolute/path/to/candidate-runtime \
  --qualification-report /absolute/path/to/passing/package-check/result.json \
  --codex-binary /absolute/path/to/codex --allow-model-inference
```

This separately invoked model check creates an isolated loopback installation
and synthetic reviewer. It does not contact a carrier or Vapi, use SMS or a
handset, or create public ingress. All Connect communication uses the installed
MCP tools and public SDK. It checks two draft artifacts, retained source facts,
server restart, exact-session resume, one correlated follow-up, explicit handled
reply consumption and an independent incoming answer. Two actual model turns
can incur model usage. Offline observer tests do not qualify those turns.

Current owner-review qualification is tracked in
[G4-034](../../docs/connect-gate-4-implementation.html#G4-034). This public runner and explicit consume behavior have fresh actual-model proof at 709cf82; historical private
observer/model results remain attributed to their recorded source.

## Native UCTP/QUIC

Create mono signed 16-bit 8 kHz WAV speech fixtures and a JSON scenario:

```json
{"turns":[{"wav":"question.wav","wait_seconds":10}]}
```

A scenario has one to six turns, waits of zero to twenty seconds, and a maximum
of 120 seconds including initial silence. Optional `wait_for_reply: true` waits
for received speech energy before the following delay, with a twelve-second
deadline included in that bound. This helps establish an actual interruption
instead of sending a second prompt before the assistant starts speaking. On macOS, `say` and `afconvert` can
produce fixtures without microphone input. Run:

```sh
python3 scripts/live/native-voice.py --allow-live-provider \
  --config .local/human.toml --assistant-config .local/assistant.toml \
  --conversation CONVERSATION_ID --assistant ASSISTANT_IDENTITY_ID \
  --scenario .local/speech/scenario.json --output .local/voice-run
```

The public SDK creates and accepts a session, sends PCMU media over native QUIC,
records return media and ends the session. Each run uses an exclusive output
directory and persists logical mutation IDs before submission. Transport
uncertainty never triggers an automatic retry. Inspect receipts before recovery.

## Browser WebRTC

Serve the public SDK's built `dist` tree and `browser-voice.html`,
`browser-voice.js`, `capture-worklet.js`, scenario and WAV fixtures on loopback.
Keep private state outside that document root. The relative SDK import assumes
this repository layout; adapt that import if staging a smaller document root.
Select an enrolled signed media configuration (including original receive/end
operations) and a same-origin scenario URL. Stage
`examples/conversation-client/canonical-media.js` with its relative SDK import.
The canonical WSS host must be explicitly installed; default startup and the old
bearer-only route do not provide this profile.

The page supplies generated audio through `BrowserVoiceSession.open`'s optional
`inputStream`, negotiates real WebRTC, records received PCM, reports ICE/codecs,
and releases the supplied tracks on hangup. It does not request a microphone.
The normal conversation example continues to use the microphone by default.

Recording upload is optional and goes through `BrowserHttpClient` to the selected
Connect conversation. It uses 32 KiB chunks, SHA-256 and stable upload mutation
IDs. Failed/uncertain uploads retain their intent; changing settings is not a
substitute for inspecting the receipt. Download the recording through the public
artifact API and verify its hash before independent speech recognition.

For interruption tests, allow the assistant to start a deliberately long answer
before speaking the interrupting fixture. Check the recording timestamps rather
than assuming a fixed wait established an overlap. A provider transcript or
nonzero RTP counter alone does not prove intelligible audio or interruption.

Phone routing and provider-initiated hangup tests require their own explicit,
bounded operator rehearsal, with saved settings and verified restoration. See
[the Telnyx SIP runbook](../../examples/agent-presence/telnyx-sip-route.md).

## Telephone and continuity rehearsal (separate from CI)

Use the checked-in `examples/agent-presence/voice.py` sample for provider-carried
telephone calls. Supply an operator-configured route, an existing conversation,
and a handset whose owner authorized the test. Save the caller receipt under
`.local/`; reuse that exact file after uncertainty. All identifiers below are
placeholders; no private account defaults are embedded in this repository.

```sh
PYTHONPATH=sdk/python python3 examples/agent-presence/voice.py start \
  --config .local/human.toml --state .local/telephone-receipt.json \
  --conversation CONVERSATION_ID --route ROUTE_ID --destination AUTHORIZED_E164 \
  --executable /absolute/path/to/connect
PYTHONPATH=sdk/python python3 examples/agent-presence/voice.py status \
  --config .local/human.toml --state .local/telephone-receipt.json \
  --executable /absolute/path/to/connect
```

For continuity, first send a distinctive fact through the SDK in that conversation.
Ask about it in the call; record the actual answer, provider/session correlation,
context selection and outcome. End the call, perform an explicitly planned server
restart, then ask a follow-up through the SDK and inspect the preserved history.
The direct Vapi telephone sample does not prove Connect-carried SIP media; use the
configured SIP route and record media evidence separately for that topology.
Record failures and missing context explicitly. Never infer recall from matching
conversation IDs or call-state success. Keep transcripts and recipient data private.

## Gate 4: a prepared task voice session

This mode uses a task and accepted audio Session that already exist. First stage
that Session with the [protected Vapi host](../../docs/VAPI_TASK_HOST.md), including
its MCP callback mapping. Opening audio then uses the existing setup. It does not
create a replacement task, conversation or voice leg.

Export a Native grant for the task with `session_invite`, `session_get`,
`session_end` and `history` inside both the task scope and grant. Give the capture
script that exported configuration; a backing operator configuration is rejected
in task mode. Current server authorization remains authoritative.

```sh
python3 scripts/live/native-voice.py --allow-live-provider \
  --runtime-root /absolute/path/to/extracted-connect-release \
  --config .local/native-task.toml \
  --task-id TASK_ID --session-id ACCEPTED_SESSION_ID \
  --conversation CONVERSATION_ID --assistant ASSISTANT_IDENTITY_ID \
  --scenario .local/speech/scenario.json --output .local/task-voice-run
```

`--runtime-root` verifies the selected binary, Python SDK and PCMU codec against
its release manifest and refuses imports from another installation. The source
script is the rehearsal driver; all client and media operations use the selected
release. Ordinary Gate 3 mode still uses `--assistant-config` and creates its
own session. These two modes cannot be mixed.

Task mode inspects the accepted audio Session through the task facade before
opening media. It ends that same Session through the facade and saves a bounded
source recall afterward. Ending the leg leaves its task and conversation intact.
Request IDs and the selected task/session are published to private intent before
side effects. Pending or uncertain controls fail without retrying; retain the
original directory and inspect receipts before recovery.

Prompts are validated before network work. Audio capture has a 120-second budget
and a matching byte cap; setup, individual SDK exchanges and cleanup have their
own deadlines. Files remain private, and results identify the selected package
or source revision, plus the rehearsal driver's file hashes. `captured` still
means only that media was recorded. Actual
MCP invocation, intelligible source use, post-call follow-through and provider
correlation require independent evidence before Gate 4 can pass.

The [local rehearsal evidence](../../docs/evidence/gate-4/task-voice-rehearsal.json)
qualifies normal and remote hangup, scoped context and the ordinary-mode
regression with both source-built and installed SDK/helper assets. It retains
failed attempts and fixes. No real provider/model acceptance is implied by
those simulator checks or by this documented command.

## Gate 4: a prepared task chat session

`task-chat.py` submits one explicit turn to the session prepared by
`connect vapi-task serve`. Keep that host and its callback mapping running.
This is a separate operator rehearsal: the model turn uses Vapi's HTTP chat API,
while task inspection and source recall use the selected Connect native SDK.
It does not create an assistant, session, task, conversation or SMS delivery.

Give the driver a private Native task configuration with `history` permission,
the existing protected setup directory and a UTF-8 prompt file of at most
16 KiB. The setup must have a verified provider session ID; reconcile an
uncertain session creation with the host command first. With `VAPI_PRIVATE_KEY`
set in the environment:

```sh
python3 scripts/live/task-chat.py capture --allow-live-provider \
  --runtime-root /absolute/path/to/extracted-connect-release \
  --config .local/native-task.toml --task-id TASK_ID \
  --setup .local/vapi-chat-private --prompt .local/chat-prompt.txt \
  --output .local/task-chat-run
```

The driver checks current task authority, saves a bounded source snapshot,
independently verifies the prepared session/assistant/nonce, and checks task
authority again before saving its dispatch intent. It submits only `sessionId`,
`input`, a unique retained `name`, and `stream: false`. Sessions configured with
a delivery transport, phone number or customer are rejected: this rehearsal
must not bypass Connect's SMS enrollment and dispatch rules.

Each output directory admits one turn. A missing or mismatched receipt leaves
that turn uncertain; repeating `capture` cannot send it again. Obtain the exact
chat ID from the provider's account/session history and inspect it without POST:

```sh
python3 scripts/live/task-chat.py reconcile --allow-live-provider \
  --runtime-root /absolute/path/to/extracted-connect-release \
  --config .local/native-task.toml --task-id TASK_ID \
  --setup .local/vapi-chat-private --chat-id EXACT_CHAT_ID \
  --output .local/task-chat-run
```

Reconciliation checks the original session, prompt and retained name against
the exact chat resource. It requires current Connect authority and never starts
another model turn. A provider session that has since expired can still be
inspected; it cannot be used for a new capture. Preserve the original output
and setup directories. A new directory is not a remedy for uncertain acceptance.

Provider requests run in independent workers with a 30-second HTTP budget,
40-second absolute worker deadline, 1 MiB response cap and no redirects/retries.
Credentials travel in the worker environment and Authorization header, never
argv or request evidence. Full responses and before/after context stay in
private files; terminal output includes only correlation and result status.
`--api-base` defaults to `https://api.vapi.ai`; an explicit loopback HTTP origin
is supported for local simulators. No provider account is used by offline tests.

`captured` confirms a matching receipt and retained context, **not** correct
reasoning, useful follow-through or an acceptance pass. Review actual MCP
invocations, sourced facts and observable Connect effects separately. Local capture and uncertain-receipt recovery pass against an actual Connect
server with both source and installed SDK/helper configurations. The evidence
is in `docs/evidence/gate-4/task-chat-rehearsal.json`; it does not itself qualify
cloud/model behavior. Separately, [actual scoped Vapi text/tool use](../../docs/evidence/gate-4/actual-vapi-chat-dc73996.json)
passes on dc73996 and [actual Connect-carried voice and post-restart continuity](../../docs/evidence/gate-4/live-voice-continuity.json)
retains its historical source and topology. Those reports preserve failed
attempts and independently verified effects/cleanup. The current owner-review
repair cycle is qualified at 709cf82 with source-linked verification through the
[Gate 4 plan](../../docs/connect-gate-4-implementation.html); a prior capture or
qualification cannot be promoted into a new pass.

Provider references: [session management](https://docs.vapi.ai/chat/session-management),
[create chat](https://docs.vapi.ai/api-reference/chats/create), and
[get chat](https://docs.vapi.ai/api-reference/chats/get).

Current owner-review qualification: Runtime 709cf82 passes 42 offline phases, 17 installed checks and 32 actual MCP calls with no errors. Qualification controls d64f1b9 combine the reviewed fixtures at 7bdd534 with all 3 hosted jobs. Local macOS arm64 owner review only; Gate 5, community publication and remote deployment remain separate.

[offline-709cf82](../../docs/evidence/gate-4/offline-709cf82.json), [package-709cf82](../../docs/evidence/gate-4/package-709cf82.json), [real-model-agent-709cf82](../../docs/evidence/gate-4/real-model-agent-709cf82.json), [review-remediation-709cf82](../../docs/evidence/gate-4/review-remediation-709cf82.json), [qualification-controls-d64f1b9](../../docs/evidence/gate-4/qualification-controls-d64f1b9.json)
