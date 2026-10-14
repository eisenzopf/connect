# Repeat the actual-agent acceptance journey

This is the separately invoked actual-model acceptance procedure for G4-034
and G5-038. The checked-in
[runner](../scripts/live/agent-journey.py) uses the selected **installed** Connect
binary, Python SDK, MCP tools and skill. It creates an isolated local server and
synthetic reviewer; it does not use Telnyx, Vapi, SMS, a handset, public ingress
or remote deployment. Two real Codex model turns use your authenticated account
and can incur model usage. Account-free CI runs only the observer's unit tests.

## Prerequisites and command

First follow [local release qualification](LOCAL_RELEASE.md) to obtain a
candidate runtime and its **passing, matching package-check `result.json`**.
Keep the original manifest/report unchanged. Use the explicit Codex executable
that passed that package's host checks; authenticate it separately using that
host's supported login mechanism. The originally qualified host version is
recorded in [the selected-host guide](CODEX_TASK_HOST.md). Qualification of one
version does not establish compatibility with every installed Codex version.
The runner records the selected host version and executable digest.

From the matching Connect source checkout, run:

```sh
python3 scripts/live/agent-journey.py /absolute/path/to/candidate-runtime \
  --qualification-report /absolute/path/to/package-check/result.json \
  --codex-binary /absolute/path/to/codex \
  --allow-model-inference
```

For Gate 5, first build a disposable integration candidate through the documented
local packaging path and finish its scripted loopback host/package qualification.
Select its runtime and matching passing report with the same command plus
`--profile gate5-local`. This integration candidate is separate from the final
G5-044 release candidate. A Gate 4 report, source test pass or scripted host result
cannot substitute for the matching Gate 5 package report. Missing authentication,
host compatibility or candidate qualification leaves the actual-model lane
unresolved; do not synthesize a passing package report to start inference.

Paths are explicit; no developer-specific application path or private harness
is required. The runner checks every installed file against its manifest and
requires that exact manifest in the passing package report before setup or
inference. It imports the installed Python client, not the checkout's SDK.
Its new private run directory is printed under `.local/gate4-agent-access/`
or `.local/gate5-agent-access/` according to the selected profile.
Credentials, model output, prompts, generated drafts and full observations stay
there. Do not commit those files or publish raw model transcripts.

## What the runner must prove

1. Create one ordinary, non-administrator agent identity and credential using
   the delivered CLI. Export its MCP connection and skill once. Record the
   installed credential/manifest digest and verify it stays unchanged.
2. The synthetic reviewer creates two independent conversations and supplies
   different project facts, random reference codes and source messages. The
   agent's prompt supplies conversation IDs and the requested workflow, **not
   those facts**. Its first actual model turn must retrieve the facts through
   Connect, create two real draft artifacts and send one tracked review question
   in each conversation.
3. An independent public SDK observer downloads both artifacts, checks their
   bytes, hashes, ownership, conversation and cited facts. It checks one question
   per conversation and two distinct waiting tasks. Model prose is not evidence
   that a tool executed or a draft was delivered.
4. Stop only the runner-owned server and restart that same installation. Verify
   the original artifacts, questions, source messages and task identities persist.
   Deliver an attributed approval for the first task and an independent ordinary
   incoming request in the second conversation. That request does not approve
   the second task.
5. Resume the **same exact saved Codex session**, with the delivered continuation
   prompt and the original credential. The model must send one source-backed,
   correlated follow-up for the first task, explicitly consume its handled reply
   through `connect_task_consume_reply`, and answer the independent request through
   ordinary `connect_send`. The second review task must remain waiting.
6. Correlate the actual MCP invocation's task/request/step/reply IDs with the
   durable journal, successful effect and message. Inspect reply attribution and
   its consumed marker through the public SDK. Replay only the original reviewer
   mutations and verify that no extra messages appear. Do not manufacture a new
   request ID to overcome uncertainty.

The runner's `result.json` is successful only when every assertion passes and
owned processes are cleaned up. It preserves unsuccessful model/tool calls as
failures even if the model later corrects them. The complete automated turn
budget is 300 seconds per turn, with an 1,100-second outer budget. Setup and
individual commands have shorter bounds. These are acceptance limits, not
promised production response times.

## Evidence and failed attempts

The public observer is split into focused modules under
[scripts/live/agent_journey](../scripts/live/agent_journey/). Reports record all
driver hashes, runtime/source identity, model executable/version, observed tool
counts, exact-session continuation, artifact/effect correlation and cleanup.
The offline tests cover manifest mismatches, explicit inference admission,
credential filtering, exact-call selection, immutable evidence and pagination.
They do not substitute for actual inference.

Review the private local-command record before publishing a redacted result.
The model may calculate artifact hashes and read its installed skill locally;
Connect communication must use its MCP tools. The command screening is a
bounded evidence check, not a sandbox or proof about arbitrary shell programs.

A failed attempt remains failed. Preserve its directory and original IDs; do
not edit a result to pass or rerun a timed-out mutation with new IDs. A fresh
attempt creates a separately identified synthetic installation. To reconcile
an earlier attempt manually, inspect its original retained receipts, model
transcripts and public journal; label that as a separate observer result and
record the original failed report's hash. The historical Gate 4 proof used
such an explicitly retained correction. Its private observer is historical
provenance, not a prerequisite for this public runner.

This journey qualifies normal agent access, optional tracking and explicit
host resume. It does not qualify unattended scheduling, a new carrier route,
voice/SMS delivery or every assistant host. Use the separate
[live voice and Vapi chat procedures](../scripts/live/README.md) for those paths;
retain their exact source versions and evidence boundaries.
