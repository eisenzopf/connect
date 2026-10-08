# Native Rust development SDK

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
