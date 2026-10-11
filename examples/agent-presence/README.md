# Give an assistant a Connect presence

This local sample talks to a real Vapi assistant through Connect's public SDK.
SMS uses that same conversation when an operator configures the human's number.
The assistant can also initiate a message: use `assistant.toml` rather than
`human.toml` and the message routes to the configured human's SMS number.

## Prepare

```sh
bash scripts/prepare-dependencies.sh
cargo build -p connect-server
(cd sdk/typescript && npm ci && npm run build)
```

Keep `TELNYX_API_KEY` and `VAPI_PRIVATE_KEY` in the server's environment.
Neither key belongs in an SDK client configuration or process argument.
Save the following plan under ignored `.local/`:

```json
{
  "name": "Connect-demo",
  "assistant_id": null,
  "phone_number_id": null,
  "callback_url": null,
  "create_sip": true,
  "outbound_voice_profile_id": null
}
```

```sh
target/debug/connect providers discover --output .local/provider-inventory.json
target/debug/connect providers plan --input .local/provider-plan.json
target/debug/connect providers apply --input .local/provider-plan.json --state .local/provider-state.json
target/debug/connect providers check --state .local/provider-state.json
```

The setup state is protected and includes SIP credentials. `apply` owns newly
created resources by a unique persisted name and records its intent before
creation. An uncertain creation is reconciled by name, never blindly repeated.
Changing the plan requires a new state path; existing assistants are never edited.
This initial plan creates an assistant and an unassigned SIP credential connection.
It does not purchase or assign a phone number, configure a usable PSTN path, or
route live audio. Those require the number, outbound profile and the voice gate.

Read `assistant_id` from the protected state and use it in local initialization:

```sh
target/debug/connect presence-init --directory .local/presence --assistant-id <assistant-id>
target/debug/connect serve --config .local/presence/server.toml --http-bind 127.0.0.1:8088 --providers
```

`presence-init` creates human/assistant identities, scoped credentials, a shared
conversation and its Vapi route. It requires a new directory. The same route can
answer an accepted native or browser audio Session; a Vapi phone number is not
required for that duplex WebSocket path. Both directions use ordinary `send`,
`history`, `inbox` and `provider_status` operations.

## Python and TypeScript

In a second terminal from the repository root:

```sh
PYTHONPATH=sdk/python python3 examples/agent-presence/chat.py \
  --config .local/presence/human.toml --manifest .local/presence/presence.json \
  --request-id demo-turn-001 --body 'Hello, what can we do together?' \
  --executable target/debug/connect

node examples/agent-presence/chat.mjs \
  .local/presence/human.toml .local/presence/presence.json \
  demo-turn-002 'Do you remember my previous question?' target/debug/connect
```

The examples require caller-owned request IDs. On uncertainty retry with the
**same ID and body**, or inspect `request_status`. Local acceptance means the
message and provider job were committed; `provider_status` distinguishes pending,
sending, accepted, completed, delivered, failed, cancelled, blocked and uncertain.
A Vapi turn completes only when its reply and chat context commit together.
History and inbox delivery are at least once; applications should save their
processed event IDs and cursor with their effects, as in `examples/review-loop`.
`provider_status` currently returns the most recent bounded page, newest first.
Delivery changes are read through this operation, not separate inbox events.
The sample is a CLI demonstration; it does not implement a persistent agent runtime.

For the separate Gate 3 route in which Telnyx SIP/RTP actually traverses
Connect before reaching Vapi, follow the
[Telnyx SIP route runbook](telnyx-sip-route.md). It preserves the current number
route, keeps Digest credentials out of SDK requests and committed files, and
requires public UDP reachability before any temporary provider switch.

## Add real SMS

Choose an existing Telnyx number **ID**, an approved human test number, and an
HTTPS URL ending in `/webhooks/telnyx` that forwards to the local server.
The callback gateway should expose only that path. The application API remains
loopback. Connect retrieves Telnyx's public verification key automatically when
`--providers` and HTTP are enabled; `TELNYX_PUBLIC_KEY` can explicitly override it.
An unavailable verifier rejects callbacks; signature checks cannot be bypassed.

For first-time provisioning only, prepare a new provider plan with the existing `assistant_id`, selected
`phone_number_id` and `callback_url`. Apply it using a **new state path**. It
creates a dedicated messaging profile and associates only the selected number.
Its prior voice and messaging settings are preserved in that private state file.
Keep `create_sip: false` unless intentionally changing that number's voice route.
An outbound voice profile is necessary for a usable outbound SIP/PSTN path;
creating a credential connection alone does not qualify voice interoperability.

If the number already belongs to an approved campaign and messaging profile,
reuse that profile. Do not move it to a newly created profile as a test shortcut.
Confirm carrier assignment and configure the approved HELP/STOP/START response
texts before a live rehearsal.

The recipient must submit the published opt-in form themselves. An operator
then reviews that evidence and activates the specific sender, conversation and
expiry through Rudeless's authenticated review service. Start the provider worker
with private `CONNECT_SMS_AUTHORITY_URL` and `CONNECT_SMS_AUTHORITY_TOKEN` values.
Connect requires a fresh grant at dispatch; neither route configuration nor an
inbound message creates consent. See [the enrollment contract](../../docs/connect-sms-eligibility.html).

Initialize a new local presence with both SMS numbers:

```sh
target/debug/connect presence-init --directory .local/sms-presence \
  --assistant-id <assistant-id> --human-number <approved-human-E164> \
  --connect-number <selected-Telnyx-E164>
```

After the reviewed grant is synchronized, send an SMS from the enrolled human
number to the Connect number. A verified
callback becomes a human message, Vapi produces a reply and Telnyx submits it.
Repeated signed callbacks produce no duplicate turn. To test assistant-initiated
contact, use `assistant.toml` with the sample and a fresh persisted request ID.
SMS is limited to 1600 characters by the adapter; longer bodies fail before
provider submission. A network failure after submission is **uncertain**, not
permission to repeat it. Disabled routes and revoked credentials stop subsequent
queued work; a request already submitted may still finish externally.
STOP persists across restart and new form submissions, cancels pending SMS, and
never becomes an assistant prompt. HELP changes no permission. START only resumes
a previously enrolled recipient; a current task grant is still required. Telnyx
sends the configured keyword acknowledgements; Connect does not duplicate them.
Inspect `provider_status` for `sms_decision` and the opaque `sms_grant_id`.

## HTTP/JSON and CLI

All application transports use the same operation JSON and authorization. HTTP
returns `{profile, result: {Ok: outcome} | {Err: error}}`; native CLI `request`
prints the outcome on success and returns nonzero on error. JSON-lines `stdio`
adds an exchange `id` and a `response` containing the same result envelope.
SDK clients load local application credentials; provider keys remain server-side.

```sh
printf '%s\n' '{"op":"health"}' | target/debug/connect request --config .local/presence/human.toml
```

Python's `HttpClient("http://127.0.0.1:8088", credential)` and TypeScript's
`HttpClient` expose the same helpers. Supply credentials from the protected local
client file, not command arguments. Native SDK clients spawn the Rust `connect
stdio` helper, which owns verified TLS and UCTP/QUIC. The HTTP mode is HTTP; neither
SDK claims a browser-native QUIC implementation. The current qualification is
macOS arm64; a portable binary or browser distribution has not been qualified.


### Dedicated numbers and SMS preflight

An operator can explicitly search with `connect providers search-numbers --country US
--output <private-inventory.json>`, then purchase the selected number with
`connect providers order-number --number <E.164> --connection-id <Connect-SIP-ID>
--state <private-order.json>`. Purchases are never performed by server startup.
Preserve the order checkpoint: an uncertain purchase is reconciled using its
persisted customer reference; it is never blindly purchased again.

`connect providers check --state <private-setup.json>` separates the messaging
profile and number association from US carrier readiness. `TCR_ACCEPTED` is still
under review; it does not mean the campaign is carrier-approved. A real campaign
in `MNO_ACCEPTED` or `MNO_PROVISIONED`, with completed number assignment, is
required by this preflight. Non-checked carrier paths remain unverified.
Recipients must have opted in to the actual sender's messaging program.

A temporary callback tunnel may point at the optional **webhook-only** listener:
`--webhook-bind 127.0.0.1:4310`. It exposes only the signed Telnyx callback path;
the HTTP application API stays on its separate local port. Unsigned callbacks
are rejected. The temporary hostname is ephemeral and its reachability alone
does not prove real SMS delivery. Configure it on the owned messaging profile,
keep the server and tunnel running for handset testing, and replace/remove the
callback when that tunnel ends.

Signed carrier-handled STOP/HELP/START, outbound echo and custom autoresponse
callbacks do not become assistant prompts. Ordinary messages such as “help with
my appointment” still enter the conversation. Delivery state is exposed through
`provider_status`; provider delivery changes are not yet separate push events.

## Incoming contact for the selected task host (under qualification)

`codex-inbox.py` is the Gate 4 incoming-contact example. It uses the public
Python native SDK and the packaged `task-access` commands. This example is
under qualification: local ledger checks and the native intake/restart check
pass against a development binary. The actual installed-host journey is not yet
qualified; retained startup timeouts are not passes. It performs no provider
setup, live SMS or deployment.

Use a separately scoped application credential with `resolve`, `capabilities`,
`inbox`, `task_create`, `task_get`, `task_grant`, and the operations it delegates.
Its recipient scope must include the configured contacts. Start Connect with an
independent loopback MCP listener and a separate application HTTP listener as
shown in [task access](../../docs/TASK_ACCESS.md). Model authentication remains
in the developer's existing Codex account configuration.

Save a private policy using the real contact identity ID and address:

```json
{
  "contacts": {"CONTACT_ID": "connect:human@local"},
  "operations": ["send", "history", "inbox", "task_cancel"],
  "task_ttl_seconds": 600
}
```

The contact sends an ordinary Connect message in an existing conversation with
this application-level body. `contact_assistant` is this example's request
format, not a new UCTP standard:

```json
{
  "action": "contact_assistant",
  "request_id": "review-request-001",
  "body": "Please review my proposal and ask me about anything missing."
}
```

Keep both the message's SDK mutation ID and the body request ID stable across
uncertainty. The example keys incoming work by sender, conversation and request
ID. An identical repeat uses the original task; changed content records a
conflict. Plain messages remain unassigned, and a message cannot choose an
existing task or expand capabilities.

```sh
PYTHONPATH=sdk/python python3 examples/agent-presence/codex-inbox.py \
  --config .local/incoming-assistant.toml \
  --policy .local/incoming-policy.json --state .local/incoming-worker \
  --http-address 127.0.0.1:8194 --mcp-address 127.0.0.1:8193 \
  --executable target/debug/connect --codex-binary codex
```

One invocation stages up to 100 inbox events and starts at most one queued
contact. Repeat the command to drain more work. This is a bounded explicit
invocation, not a background watcher. The app commits its cursor alongside
queued work before dispatch, saves the original task creation intent, and uses
the incoming journal event as `initiating_event_id`. It preserves the original
conversation and independently tracks overlapping requests. Configured tools
run autonomously without per-action permission prompts.

The output distinguishes a processed page, a closed task and a completed host
turn. A completed host turn does not imply task completion. A lost app
acknowledgement after the host checkpoint does not start an extra turn. Later
bound replies continue through the documented exact-session `run-codex`
command and the returned installation path. Automatic reply-triggered wake is
not implemented here. Owner-issued grant renewal uses `renew-codex`; expired
or interrupted issuance never silently receives a fresh ID or lifetime.

Keep the entire private state directory. It contains the cursor, source records,
original requests, grants, host installation and logs. A changed identity/policy,
unknown database schema, incomplete export or corrupt checkpoint requires
explicit repair. Do not delete it to retry an uncertain send. State is bounded
to 10,000 journal records. Current Connect authority is checked again on replay.

Run the local example checks separately from provider tests:

```sh
PYTHONPATH=sdk/python python3 -m unittest discover \
  -s examples/agent-presence/tests -p 'test_incoming*.py'
CONNECT_TEST_BINARY="$PWD/target/debug/connect" PYTHONPATH=sdk/python \
  python3 -m unittest discover -s examples/agent-presence/tests \
  -p 'test_public_intake.py'
```

The public-process check requires an explicitly selected built Connect binary;
its absence fails. The unit checks alone do not establish an installed host or
model journey. Process cleanup beyond the packaged host's documented ownership
boundary remains subject to the [host limitations](../../docs/CODEX_TASK_HOST.md).

The opt-in incoming runtime check uses real `run-codex` processes and an isolated
scripted Responses peer (no inference or model account):

```sh
CONNECT_TEST_BINARY="$PWD/target/debug/connect" \
CONNECT_TEST_CODEX_BINARY="$(command -v codex)" PYTHONPATH=sdk/python \
  python3 -m unittest discover -s examples/agent-presence/tests \
  -p 'test_installed_incoming.py'
```

It retains synthetic private fixtures under `.local/incoming-runtime/` for
failure inspection. No actual-model, cold-start, installed release or full
Gate 4 qualification is implied by the native intake checks.
