# Connect development SDKs

`connect-client` is the Gate 1 Rust SDK. It uses existing rvoip UCTP/QUIC with
mandatory certificate/hostname verification. The WS parity peer is a test
harness, not this SDK or a TypeScript native package.

The configuration, native exchange lifecycle, typed errors and application
facade have separate modules. Consumers import protocol types from
`connect_client`; they do not need the server or storage crate.

| Method | Result / behavior |
|---|---|
| `open(ClientConfig)` | Verified TLS, required profile and credential-subject binding |
| `resolve(address)` | Scoped durable identity and declared capabilities |
| `create(request_id, participants)` | Saved conversation, caller included |
| `send(request_id, conversation_id, body)` | Durable `SendReceipt`; not proof of peer processing |
| `history(conversation_id, after, limit)` | Ordered `HistoryPage`; follow next until empty |
| `inbox(cursor, limit, wait_ms)` | Resumable `InboxPage`; at-least-once events |
| `request_status(request_id)` | Accepted saved outcome, Rejected, or NotFound |
| `close_conversation(request_id, conversation_id)` | Blocks new sends; history/receipts remain |
| `health()` | Network installation/profile/schema facts |
| `request(Operation)` | Full typed application operation |
| `request_cancelled(operation, token)` | Explicit cancellation; closes physical peer |
| `close()` | Bounded physical-peer shutdown |

Client requests are serialized per client. Timeout/cancellation closes that
physical peer; reopen it to continue. A transaction might already have committed,
so retain the original logical ID and inputs. The SDK does not silently retry or
mint replacement IDs. `NotFound` is not proof a concurrent uncertain request
cannot still finish. Exact replay and request lookup use current authority.

Configuration timeout is bounded to 1–60,000 ms per exchange (10,000 default).
An inbox wait is at most 5,000 ms. The caller may choose shorter limits; an inbox
wait longer than its deadline predictably returns a timeout. Trust material is
required. This local profile accepts loopback endpoints only. The config's stable
identity metadata does not override the server's authenticated sender; examples
also resolve their own alias and verify its ID before opening their work state.

Persist mutations **before** sending. Persist each business effect, handled event
ID, any pending reply command and cursor atomically in the app's own store.
A restored server snapshot may be older than the app cursor; that is an explicit
error requiring reconciliation, not an instruction to reset app history.

A compiling native example is in the crate's Rust documentation; the runnable
process journey is [examples/review-loop/README.md](../examples/review-loop/README.md).
Verify them with:

```sh
cargo test --locked -p connect-client --doc
cargo test --locked -p connect-server --test sdk_facade
python3 scripts/check-examples.py
python3 scripts/check-example-recovery.py
```

The deterministic examples prove the transport/recovery interface without
provider keys. MCP, the installable agent skill, Vapi, Telnyx and an external
assistant are later v0.1 gates; Gate 1 does not claim those integrations.

## Gate 2 interfaces

The Rust client also exposes `provider_status(conversation_id)`. The TypeScript
SDK is in `sdk/typescript`; Python is in `sdk/python/connect_sdk`. Both provide
`NativeClient` through a persistent Rust helper, and a separate loopback
`HttpClient`. They share identities, logical request IDs, outcomes and cursors.
Neither uses WebSocket as a substitute for native QUIC. These are development
sources, not published npm/PyPI packages.

The messaging sample and provider setup instructions are in
[examples/agent-presence](../examples/agent-presence/README.md). Gate 2 requires real Vapi chat and telephone voice in both directions. Live SMS
and its signed carrier callback proof are explicitly deferred pending approval.
MCP tools and the installable skill are Gate 4; HTTP/JSON alone is not MCP.

## Gate 2 telephone call control

Implemented methods are `call_start(request_id, conversation_id, route_id, destination)`,
`call_status(conversation_id, call_id)` and
`call_end(request_id, conversation_id, call_id)`. TypeScript uses camelCase
method names; Python and Rust use snake_case.

They control Vapi telephone calls; they do not carry client audio. New voice
routes restrict destinations and bind a Vapi assistant/phone ID to a conversation.
Voice credentials require an explicit grant; messaging credentials do not gain
voice access. Call receipt replay must not redial, and uncertain creation must
not be automatically repeated. Private provider control URLs remain server-side.

[Voice samples](../examples/agent-presence/voice.md) include persistent request
receipts and inbound setup. The supported Vapi native Telnyx integration provides
the Gate 2 telephone route. The separately tested BYO SIP credential path still
returns Vapi HTTP 400; native SIP and QUIC audio remain Gate 3.

Schema 6 preserves call direction, inbound bindings, discovery watermarks and
the credential that requested a call end. End intent survives concurrent polls
and restart; an uncertain end is reconciled by reads rather than blindly repeated.
Dispatch rechecks the requesting credential's current voice authority.

Inbound records appear through the existing inbox after authenticated polling.
Discovery scans bounded updated-time windows with pagination, provider-ID
deduplication and a durable watermark. See the sample for scan limits and delay.
Caller ID is a routing hint, never an authorization grant. Current verification
and live audio results are linked from the Gate 2 implementation checklist.
