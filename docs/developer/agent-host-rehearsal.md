# Local agent-host transition rehearsal

G6-065 needs an actual supported agent host to request and observe a transition.
The protocol-client tests remain separate evidence. The explicit rehearsal worker
uses the same private native handoff scene, with independent signed QUIC speakers,
actual marked PCMU, excluded-audience checks, original receipts and owned cleanup.
There are no PSTN, SMS or provider/model voice endpoints in this scene.

The current source-compatible launcher profile is `codex-cli 0.164.0-alpha.11.1`.
Version, `exec`/`resume` flags, MCP configuration parsing and local account status
can be checked without starting a model. Those checks do not qualify a model turn.
Unknown versions and the historical host pin are refused. Codex's documented
[non-interactive JSONL mode](https://learn.chatgpt.com/docs/non-interactive-mode)
and [MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
provide the host boundary; Connect retains its own private intent/session state.

## Procedure

Prepare dependencies and build the server library and the
`gate6_canonical_runtime` integration target with `test-support`. Use the normal
qualified release build procedure and retain the build/source hashes. Select the
sealed test executable explicitly; never select a prior binary by filename alone.

Create a fresh private JSON configuration under ignored `.local/`. Its output
parent must already exist, and the output directory must not exist. For the
account-free protocol rehearsal:

```json
{
  "profile": "connect-agent-host-rehearsal/1",
  "mode": "protocol",
  "output": "/absolute/checkout/.local/agent-host/protocol-run",
  "timeout_seconds": 120,
  "approved_model_execution": false,
  "codex_binary": null,
  "codex_sha256": null
}
```

Run the explicitly ignored entrypoint with the selected configuration:

```sh
CONNECT_AGENT_REHEARSAL_CONFIG="$PWD/.local/agent-host/protocol.json" \
  /absolute/sealed/gate6_canonical_runtime \
  canonical_agent_rehearsal::native_handoff_worker \
  --exact --ignored --test-threads=1 --nocapture
```

The worker exports a fresh ordinary credential with only `session.handoff` and
`operation.get`, the MCP configuration and the reviewed Connect skill. It creates
an exact protected prompt from the current scene, preserving its operation ID,
resource scope, revisions and complete payload. Temporary export-source and
server configuration files are removed before a host can start. Native signing
keys and administrator/provider credentials are absent from the installation.
The retained native state and operator configuration live in a private sibling
directory under the selected output, with exact listener addresses recorded for
explicit recovery. They remain available after either success or failure.

Protocol mode submits through the official RMCP client and cannot launch a model.
Its `host-probe.json` and `outcome.json` explicitly record `actual_model_turn:false`.
It is a rehearsal of the installation and media path, not agent-host acceptance.
A Codex configuration without explicit approval fails before server/fixture setup.

## Actual Codex lane

Before invoking this lane, record the selected binary/hash, existing authenticated
Codex account reference, exact prompt, 1–300 second turn budget and cleanup in the
bounded rehearsal configuration required by G6-093. Obtain the owner's explicit
agent-execution authorization. The product plan states that the plans do not
authorize agents or live calls by themselves. Do not interpret a protocol pass as
that authorization.

Use a fresh configuration/output, `mode:"codex"`,
`approved_model_execution:true`, and the selected absolute `codex_binary` plus its
SHA-256. The source-qualified launcher still checks the exact version and local account
status. No model/provider credential is copied from the account files.
The existing launcher supplies only the scoped Connect bearer to the child and
retains/redacts the actual JSONL stream and exact resumable session.

The intended turn discovers its tools, submits one exact handoff, then observes
that original operation. It must not invent another mutation or report audio
from metadata. Qualification requires an actual completed Codex turn, exactly one
matching `connect_agent` handoff tool item and a status tool item in its real
JSONL stream. The independent native assertions and joined cleanup must also pass.
Unexpected event formats or missing calls are failures, not protocol-mode passes.

On timeout, unknown outcome or failed cleanup, keep the original private state,
model session, prompt, operation and process handle for inspection/reconciliation.
Do not start another model session or repeat the mutation to conceal uncertainty.
Publication, remote/provider/telephone/browser profiles and Gate 6 acceptance
remain separate actions and evidence.


## Recorded actual host result

On October 26, 2026 the owner explicitly authorized one 120-second Codex
0.164.0-alpha.11.1 rehearsal on the existing local account. The current sealed
worker passed in 47.6 seconds. Its real JSONL contains one exact handoff tool call
and one exact original-operation status read, returning `completed`. No shell,
file or unrelated MCP tool items occurred. The model reported metadata and
explicitly declined to claim decoded-audio verification.

Independent native assertions passed for marked decoded PCMU, audience and role
handoff, observer publication refusal, continued media after read expiry,
original receipt replay, one physical attempt and joined cleanup. The native
QUIC, HTTP3 and MCP HTTP listener addresses were reclaimable afterward. The
original prompt/session, native state and redacted JSONL remain under ignored
`.local/`; no replacement or resumed model turn was started.

See [the bounded actual-host evidence](../evidence/gate-6/canonical-agent-host-authorized.json).
This fulfills the actual supported-host request/observe subrequirement locally.
Remaining facade/profile work, remote/provider/telephone/browser qualification
and Gate 6 acceptance remain open. This one authorization does not authorize
another model rehearsal.
