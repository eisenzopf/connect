# Local telephone call sample

The Connect server accepts and journals a call before its worker contacts Vapi.
Vapi runs the assistant and uses its supported Telnyx integration for telephone audio.
This sample exercises call control over native UCTP/QUIC; it does not yet send
microphone audio through QUIC datagrams.

Prepare an existing Connect-owned number with `connect providers voice-apply --inbound-only`.
This command reuses the protected checkpoint from `providers apply`, creates an
isolated outbound profile and inbound SIP route, and retains the old number
configuration privately. It never purchases a number. Its current gateway
configuration targets Telnyx's US SIP gateways documented by Vapi.

For provider telephone voice, complete setup with:

```sh
connect providers voice-native --voice-state .local/demo/voice-setup-state.json \
  --state .local/demo/native-voice-state.json
```

The input is the protected Connect voice checkpoint created by `voice-apply`.
The native command reads `TELNYX_API_KEY` and `VAPI_PRIVATE_KEY` from its environment,
creates a named Vapi Telnyx credential, imports the selected owned number, assigns
its assistant and verifies the number/application/assistant mappings. It saves
creation intent before effects and recovers pending resources by name. It buys
no number and preserves the SMS messaging profile.

Vapi may reuse an account's existing Telnyx application. Connect detects a shared
application and uses its current outbound profile without changing it. Only an
unshared application is assigned the Connect-owned profile. The output reports
`provider_application_shared`, the actual profile and `vapi_phone_number_id`;
use that native phone ID for both outbound routing and inbound discovery.
The BYO SIP credential alternative remains available, but this account's Vapi
validation returns HTTP 400. That failure is not represented as qualified SIP.

On a stopped local installation, use `connect voice-route` with a private JSON
binding containing `id`, `conversation_id`, `identity_id`, `assistant_id`,
`phone_number_id` (the Vapi ID), `destinations` (explicit E.164 test destinations),
and `enabled`. Issue an application credential with `connect issue --voice`.
Existing messaging credentials remain messaging-only. Start the local server
with `--providers`; it reads `VAPI_PRIVATE_KEY` from its environment.

```sh
PYTHONPATH=sdk/python python3 examples/agent-presence/voice.py start \
  --config .local/demo/voice-client.toml --state .local/demo/voice-request.json \
  --conversation CONVERSATION_ID --route assistant-voice --destination YOUR_E164_NUMBER

PYTHONPATH=sdk/python python3 examples/agent-presence/voice.py status \
  --config .local/demo/voice-client.toml --state .local/demo/voice-request.json

PYTHONPATH=sdk/python python3 examples/agent-presence/voice.py end \
  --config .local/demo/voice-client.toml --state .local/demo/voice-request.json

CONNECT_EXECUTABLE=target/debug/connect node examples/agent-presence/voice.mjs \
  start .local/demo/voice-client.toml .local/demo/voice-ts-request.json \
  CONVERSATION_ID assistant-voice YOUR_E164_NUMBER
```

Build the TypeScript SDK first with `npm run build` in `sdk/typescript`. The Rust
SDK exposes the same `call_start`, `call_status`, and `call_end` methods. Python
`HttpClient` and TypeScript `HttpClient` expose the same operations over the local
HTTP/JSON facade. The CLI `connect request` accepts their operation JSON on stdin.

Reuse the same receipt file after a timeout. Never create another request ID to
resolve an uncertain provider creation: doing so may dial twice. `pending` is
local acceptance, not proof of ringing. `ended` is provider status, not a human
assessment of audio quality. Vapi's private control URL remains server-side.
SMS consent and carrier approval are independent of a voice test.


## Inbound telephone calls

The setup command also supports `--inbound-only`. This configures the owned
Telnyx number and Vapi SIP address without claiming the outbound credential is
ready. The returned `inbound_configured` and `configured` flags are distinct.
Record its `vapi_sip_number_id` separately from an outbound phone binding.

On the stopped local installation, bind the inbound SIP phone ID:

```sh
connect voice-ingress --config .local/demo/operator.toml \
  --server-config .local/demo/server.toml --route assistant-inbound \
  --phone-number-id VAPI_SIP_PHONE_ID
```

Use a separate `assistant-inbound` voice route when only inbound provisioning is
ready. Its `destinations` list contains the permitted caller numbers. Existing
messaging credentials can observe the inbox; explicitly grant voice control
only when needed. A provider-reported caller number is a routing hint, not proof
of identity or authorization to execute an assistant action.

The worker discovers calls through authenticated Vapi REST reads every five
seconds. It imports only inbound calls created after route activation that match
the configured assistant, Vapi phone ID, and caller allowlist. An unknown caller
can still reach the provider assistant, but is not guessed into a private Connect
conversation. Repeated discovery is deduplicated by provider call ID; existing
call lifecycle polls update immutable journal events. Call events carry
`direction: inbound` or `outbound`. Provider control URLs remain private.

Discovery uses updated-time windows, with a fixed upper bound two seconds behind
the scan start and a 60-second overlap on the saved watermark. Saturated pages
are subdivided until complete, deduplicated by provider call ID, and committed
before advancing the watermark. More than 100 lifetime calls no longer blocks
new calls. Each batch permits 128 provider reads; an unsplittable millisecond
with 100 records or an exhausted scan budget warns and preserves the cursor for
reconciliation. This remains a bounded local test implementation. Polling and
provider indexing delay local state; this is not a real-time media guarantee.
Discovery and control have separate lifecycle-owned workers, so a slow discovery
request does not hold up a call start or end.

Schema 6 adds scan watermarks and attributable end intent to schema 5's inbound
bindings/direction. Use a stopped installation, fresh verified backup and
`connect migrate` for schema 1–5 state. The tests preserve prior state and the
original backup. Accepted end intent survives a racing status poll and restart;
its actual requester's current voice authorization is checked at dispatch. An
uncertain end is reconciled by reads without blindly repeating the mutation.

## Current live test status

See `docs/connect-gate-2-implementation.html` and the linked evidence for the
current result. SIP setup, HTTP 200, and an `ended` call record do not establish
intelligible two-way telephone audio. Native QUIC microphone/speaker media and
WebRTC have separate Gate 3 acceptance.
