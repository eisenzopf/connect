# M3 — native application traffic and loopback WS parity

Commands: `cargo test --locked -p connect-server --test contract_fixtures
--test transport_parity` (2 fixture + 4 wire tests passed), and `cargo test
--locked -p connect-server --test transport_parity --test transport_pressure`
(4 parity + 3 pressure/authority tests passed). Strict library/binary clippy
passed with `-D warnings`. Source/dependency/environment identities are recorded
in the local Git checkpoint, source lock and environment report.

- Every positive operation fixture executes on both physical transports,
  authenticated through rvoip. Replies preserve profile, envelope version,
  correlation and result shape. Mutation replay across transports returns the
  saved result; history and request lookup agree on the same durable facts.
- All eight unknown-actor-field negative envelopes fail on both wires.
  Anonymous operations, unsupported profiles and unsupported envelope versions
  fail explicitly. Sender identity comes from the retained validated credential.
- Native and WS clients receive equivalent foreign-resource/discovery denials.
  Active revocation and fresh-handshake revocation fail on both paths. A new
  narrower credential cannot reveal an older saved send receipt. Foreign
  identity cursors are rejected; expired credentials fail existing/fresh native
  access and the active/fresh WS auth lifecycle.
- Cross-transport sends wake the other transport's waiting inbox through one
  Service instance. Reliable read pages cap serialized entries at 512 KiB;
  twenty maximum-size escaped bodies require multiple physical pages and every
  message is recovered, without skipping or exceeding wire framing.
- Slow native and WS readers are flooded with 600 history requests each. The
  native inbound queue reaches its fixed 256-entry bound; WS aggregate output
  high-water remains within its 256-entry bound. A healthy peer concurrently
  commits twenty messages. Reconnecting and paginating in pages of three
  recovers all 21 messages, including the seed, in order. This qualifies bounded
  queues and recovery, not a production RSS/load/throughput guarantee.
- A one-peer limit refuses an extra connection on each listener while the
  existing peer remains usable. The configured cap is per listener. Shutdown
  drains peers and the exact former WS port can immediately bind again.

The public Rust client uses actual native QUIC and verified TLS. Raw fixture
peers are test support, not another SDK. Optional WS is plaintext loopback-only;
decision 005 explains its coordinator reuse and owned local lifecycle. No live
provider, media interoperability, WAN or production claims follow from M3.
