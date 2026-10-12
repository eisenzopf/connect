# 002 — SQLite is the authoritative local application store

A single transaction commits each accepted operation, its request digest/result,
message or conversation change, and every intended recipient event. rvoip is
not used as a second application database or mutation executor. Journal reads
recover all accepted events after loss of live notification hints.

Use a serialized SQLite writer, WAL and synchronous FULL. Each request applies
current credential validity, operation scope, participant scope and membership
before querying cached results. No expiry/retention cleanup is enabled initially.
The store refuses unknown newer schemas and reports underlying failures through
stable redacted errors. App-side job deduplication remains the app's responsibility.

Limits and units are defined in `connect_protocol::Limits` and the profile
contract. Initial listeners are loopback UDP 4433 / optional WS TCP 8088;
tests select ephemeral ports. Client requests have a 10 s deadline; inbox waits
are at most 5 s. Upstream wire input limits and queues remain bounded. Initial
qualification is macOS arm64, local connections only.

## Limit qualification cases

| Limit | Unit / default | Required case / failure |
|---|---|---|
| message_bytes | bytes / 16,384 | accepted at bound; oversize invalid-request before commit |
| page_size | events/messages / 100 | zero/excess invalid-request; bounded pagination recovery |
| participants | identities / 8 | duplicate normalization and over-bound invalid-request |
| journal_events | durable event rows / 100,000 | concurrent writers cannot overshoot; capacity-exceeded |
| operation_receipts | durable receipt rows / 100,000 | new requests rejected at capacity; saved replays still recover; no receipt falsely claimed |
| message_storage_bytes | retained UTF-8 bytes / 64 MiB | serialized capacity rejection preserves earlier history |
| wait_ms | milliseconds / 5,000 | bounded empty wait; boundary insert remains discoverable |
| request_timeout_ms | milliseconds / 10,000 | timed out request keeps caller logical ID; shutdown remains bounded |
| connections | simultaneous peers per listener / 64 | excess connection refusal; existing peer remains usable |
| serialized page | bytes / 512 KiB | fewer than requested entries allowed; safe cursor continuation below 1 MiB wire cap |
| upstream queue | envelopes / 256 | stalled output cannot grow memory without bound; durable catch-up |

Configuration rejects zero/unbounded limits where applicable. M2–M5 execute
the storage, wait, transport and capacity cases; this decision defines those
cases and does not claim all their checks have run at M0.
